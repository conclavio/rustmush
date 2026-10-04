//! **The native stock mind** (17b cut M1a.6; doc 17 §17 "Migrating `reflex.rs`
//! into Standard Doctrine", D-UM-3, D-UM-48; the M1a plan's §6).
//!
//! T1 of doc 17 §17's three columns: Standard Doctrine — the reflex
//! controller's state machine, its utility score, its goal precedence, its
//! clocks and its orders' obedience — written in Rust over a [`MindView`] and
//! a [`Memory`] instead of over the arena, and flying the hull through the
//! mind schedule (`DueKind::MindThink`/`MindLand`) and the motor
//! ([`crate::arena::motor`]) instead of a `ReflexTick`. It is the oracle the
//! Rhai port (T2, 17b cut M2) is held to byte for byte, and the measured
//! native fast path P3 allows. [`crate::arena::mind::default_mind_runner`]
//! answers it.
//!
//! # `decide`, re-expressed arm for arm
//!
//! [`NativeStockMind`]'s think is `crate::arena::reflex`'s `decide`, read with
//! its comments beside it and ported block by block **in the same order**:
//! the `Disabled` edge; the standing order and its three early ends (16b cuts
//! AA.1, O.1, Z.2); the power arms; the `lost` edge; the order's steering; the
//! two `Withdraw` edges; the hostile edges with the utility score
//! ([`best_hostile`]) and the supporter's leash; the `no line` edge; the
//! states' bodies — `Patrol`'s list, `Approach`, `Engage`, `Withdraw` with its
//! stand, dwell and rally — and the search's and the hunt's bookkeeping. Where
//! `decide` wrote a [`ReflexPlan`](crate::arena::reflex) it emits intents, and
//! where it wrote the hull's `MotorRow` it emits a goal:
//!
//! * **`Goal(Cell)` only when the goal CHANGED** — the memory's `goal` is the
//!   goal the mind believes the motor holds — so the motor keeps its route
//!   between thinks;
//! * **`Goal(Hold)`** where `decide` stopped the hull (`speed 0`): the motor
//!   lets its goal go and the hull stops (the memory's `goal` is null);
//! * **arrival** is the hull's own cell within [`ARRIVE_CELLS`] of the goal,
//!   **or** a `drive_arrived` notice since the last think for the memory's
//!   goal: the motor's fan (the M1a plan's departure (d)) may have landed the
//!   hull on another square than the one named, and the mind reads the
//!   motor's word for it;
//! * a **`drive_no_route`** notice for the memory's goal performs the
//!   reflex's `unreachable` edge in its order — the `Advance` and `Support`
//!   ends, the search's and the hunt's give-ups, the Hold retry — and then the
//!   waypoint fallback ([`fallback_waypoint`]) as the next goal, one think
//!   later than the reflex's in-tick fallback (departure (i); the divergence
//!   `fallback`).
//!
//! # The emission order, and its two rules
//!
//! `Startup`, `Goal`, `Heading`, `Speed`, `Lock`, `Fire`. A think that emits
//! `Goal(Cell)` emits **no** `Heading` or `Speed`: the motor steers from the
//! landing on, and the landing's disengage rule (a mind's own heading lets go
//! of its goal, 17b cut M1a.5) would throw the goal away. A think that
//! **stands** emits `Goal(Hold)` first and then the facing `Heading` — *stop
//! here and face there*; the goal is already gone, so the disengage rule has
//! nothing to let go of. The stock mind never emits a `Speed` at all
//! (`Goal(Hold)` is its stop), answers `Startup` alone, and so never more than
//! four intents (`Goal(Hold)`, `Heading`, `Lock`, `Fire`), well inside
//! [`crate::arena::mind::MIND_INTENTS_MAX`].
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
//! Three residuals, recorded for 17b cut M4: the view carries no authored
//! speed, so a hull authored with none reads as able to close (`immobile` is
//! the view's `cannot_close`); no cost row, so every picker is asked for
//! [`CostRow::Tracked`] (the twin lane fields stock line tanks only); and no
//! match mode, so a think does not know a Hold match from a skirmish and the
//! Hold arms below run only for a caller that hands them the objective —
//! this module's own tests do, and no production think can until the mode is
//! on the view.
//!
//! # No draw, no clock
//!
//! It draws nothing: every choice is a total order over integers, and the walk
//! keys on the view's own `unit` (departure (e)). It reads no wall clock; the
//! twin lane times it from the test.

use mush_atlas::Cover;
use smol_str::SmolStr;

use crate::arena::damage::{BLOCKED, COVER};
use crate::arena::intents::{IntentKind, MotorGoal};
use crate::arena::mind::{
    Memory, MindRunner, MindView, ThinkJob, ThinkResult, ViewContact, decode_memory, encode_memory,
};
use crate::arena::motor::{
    ARRIVE_CELLS, Walk, entry_cell, fallback_waypoint, far_entry_cell, hold_arrived, patrol_step,
    stand_off_cell,
};
use crate::arena::perception::{Aspect, DriveOutcome, FrameNotice, FrameOrder, MindObjective};
use crate::arena::reflex::{
    EXPOSURE_FLANK, EXPOSURE_FRONT, EXPOSURE_REAR, REFLEX_STAND_TICKS, Ranges, ReflexState,
    SCREEN_HALF_ARC_MDEG, SUPPORT_LEASH_CELLS, SUPPORT_STATION_CELLS, arc_between, stand_off_want,
};
use crate::arena::state::{
    ArenaDefinition, Classification, ContactId, CostRow, DEFAULT_SENSOR_HEIGHT_CM, HoldState, Power,
};
use crate::arena::weapons::{FrameWeapon, fight_inner_cm_of, fight_limit_cm_of, loaded_weapon_of};
use crate::config::ReflexConfig;
use crate::deliberation::order::RangeBand;

// ---------------------------------------------------------------------------
// The memory (D-UM-30's shape)
// ---------------------------------------------------------------------------

/// **The fourteen keys of the stock mind's memory** (17b cut M1a.6), in the
/// order [`StockMemory`]'s fields are declared — and nothing else: a test
/// pins the list.
pub const STOCK_MEMORY_KEY_NAMES: [&str; 14] = [
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
    "ignore_order",
];

/// **What the stock mind remembers between thinks** (17b cut M1a.6; doc 17
/// §8) — a typed view over its [`Memory`], read with `StockMemory::from` and
/// written back with `Memory::from`, so what the arena stores is the
/// canonical map of [`STOCK_MEMORY_KEY_NAMES`] (D-UM-30) and 17b cut M2's Rhai
/// port can write the same bytes.
///
/// The reflex's [`crate::arena::reflex::ReflexUnit`] and the driving half of
/// its `MotorRow` that the decision reads, field for field: the motor's own
/// route, search stamp and stops are the motor's and are not remembered here.
/// Two fields are new: `goal` is the goal the mind believes the motor holds
/// (null once it let the hull hold), and `ignore_order` is how a mind ends an
/// order it cannot journal the end of (the reflex's 16b cuts AA.1, O.1 and
/// Z.2 ends; the arena's own expiry still comes).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StockMemory {
    /// `state`: where it is in the machine ([`ReflexState::code`]).
    pub state: ReflexState,
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
    /// `ignore_order`: the standing order is over for this hull, which no
    /// longer obeys it; cleared when the view carries no order.
    pub ignore_order: bool,
}

/// An integer key, or `None` when absent, null or not an `i64`.
fn int_at(memory: &Memory, key: &str) -> Option<i64> {
    memory.get(key).and_then(serde_json::Value::as_i64)
}

/// An array key of integers, or `None`.
fn ints_at(memory: &Memory, key: &str) -> Option<Vec<i64>> {
    memory
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_i64)
                .collect::<Vec<i64>>()
        })
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
fn cell_value(cell: Option<(u16, u16)>) -> serde_json::Value {
    cell.map_or(serde_json::Value::Null, |(x, y)| serde_json::json!([x, y]))
}

/// An integer, as its memory value.
fn int_value(value: Option<i64>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, serde_json::Value::from)
}

/// The state a code names, `Startup` for none.
fn state_of(code: Option<i64>) -> ReflexState {
    ReflexState::ALL
        .iter()
        .copied()
        .find(|state| Some(i64::from(state.code())) == code)
        .unwrap_or_default()
}

impl From<&Memory> for StockMemory {
    /// Read the fourteen keys; an absent, null or ill-shaped key is the
    /// field's default (an empty memory is a fresh sortie).
    fn from(memory: &Memory) -> StockMemory {
        let flag = |key: &str| {
            memory
                .get(key)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
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
            ignore_order: flag("ignore_order"),
        }
    }
}

impl From<StockMemory> for Memory {
    /// Write all fourteen keys, a null for every field that holds nothing —
    /// so one memory has one encoding, whichever think wrote it.
    fn from(stock: StockMemory) -> Memory {
        let mut memory = Memory::new();
        let mut put = |key: &str, value: serde_json::Value| {
            memory.insert(SmolStr::new(key), value);
        };
        put(
            "state",
            serde_json::Value::from(i64::from(stock.state.code())),
        );
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
            stock
                .last_plot
                .map_or(serde_json::Value::Null, |((x, y), at)| {
                    serde_json::json!([x, y, at])
                }),
        );
        put("unheld_since_ms", int_value(stock.unheld_since_ms));
        put("quiet_since_ms", int_value(stock.quiet_since_ms));
        put("stand_until_ms", int_value(stock.stand_until_ms));
        put("hold_retry_ms", int_value(stock.hold_retry_ms));
        put("rallied", serde_json::Value::Bool(stock.rallied));
        put("hunt", cell_value(stock.hunt));
        put("no_line", serde_json::Value::Bool(stock.no_line));
        put("ignore_order", serde_json::Value::Bool(stock.ignore_order));
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

/// **Armour plus internals, per mille** (the reflex's
/// `reflex_health_permille`, over the view's own sections): every facet's
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
        matches!(row.class, Classification::Classified { team } if team != view.team)
            && row.reads_dead != Some(true)
    })
}

/// **The three distances a think fights at** (the reflex's `ranges`, over the
/// view: the loaded weapon's band edges, its rounds and the view's sight in
/// whole metres — the divergence `range_round`).
#[must_use]
pub fn view_ranges(view: &MindView, cfg: &ReflexConfig, order: Option<&FrameOrder>) -> Ranges {
    let loaded = loaded_weapon(view);
    let bands: &[u32] = loaded.map_or(&[], |weapon| weapon.bands_cm.as_slice());
    let sight = sight_cm(view);
    let order_cap = order
        .and_then(|order| order.max_range_band)
        .map_or(u64::MAX, |band| band.ceiling_cm(bands, sight));
    let cap_inner = fight_inner_cm_of(bands.first().copied(), sight).min(order_cap);
    Ranges {
        engage_cm: cap_inner.saturating_mul(u64::from(cfg.engage_band_permille)) / 1_000,
        release_cm: cap_inner,
        stand_cm: RangeBand::Close
            .ceiling_cm(bands, sight)
            .min(order_cap)
            .min(cap_inner),
        limit_cm: fight_limit_cm(view),
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
/// hull is absent: D-UM-48).
#[must_use]
pub fn best_hostile_ranked(
    view: &MindView,
    cfg: &ReflexConfig,
    health_permille: u32,
    prefer: Option<ContactId>,
    admit: impl Fn(&ViewContact) -> bool,
) -> Vec<(u64, ContactId)> {
    let reach = fight_limit_cm(view);
    if reach == 0 {
        return Vec::new();
    }
    let health_term = cfg.weight_health.saturating_mul(health_permille.min(1_000));
    let mut ranked: Vec<(u64, ContactId)> = view
        .contacts
        .iter()
        .filter(|row| {
            matches!(row.class, Classification::Classified { team } if team != view.team)
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
                u64::from(cfg.weight_range.saturating_mul(range_permille))
                    .saturating_add(u64::from(cfg.weight_facet.saturating_mul(exposure)))
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
    cfg: &ReflexConfig,
    health_permille: u32,
    prefer: Option<ContactId>,
    admit: impl Fn(&ViewContact) -> bool,
) -> Option<ContactId> {
    best_hostile_ranked(view, cfg, health_permille, prefer, admit)
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
fn line_between(def: &ArenaDefinition, own: Option<(u16, u16)>, cell: Option<(u16, u16)>) -> bool {
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

/// The view's own pose, `(x_cm, y_cm, z_cm)`.
fn own_point(view: &MindView) -> (i64, i64, i32) {
    (
        view.x_cm,
        view.y_cm,
        i32::try_from(view.z_cm).unwrap_or(i32::MAX),
    )
}

/// The stand-off cell `want_cm` back from a row's plot cell, as a tracked hull
/// (`None` for a row whose plot is off the window).
fn stand_off_of(
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

/// The row a contact id names, if the view holds it.
fn contact(view: &MindView, id: ContactId) -> Option<&ViewContact> {
    view.contacts.iter().find(|row| row.id == id)
}

/// Whether `a` is within [`ARRIVE_CELLS`] of `b` (Chebyshev).
fn near(a: (u16, u16), b: (u16, u16)) -> bool {
    a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)) <= ARRIVE_CELLS
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

/// **Whether an `Advance { to }` has reached the Hold square** (the reflex's
/// `advance_reached_the_hold`): `to` inside the square, and the hull inside
/// it too or within [`ARRIVE_CELLS`] of `to`.
fn advance_reached_the_hold(hold: &HoldState, to: (u16, u16), here: Option<(u16, u16)>) -> bool {
    hold.square_contains(to)
        && here.is_some_and(|cell| hold.square_contains(cell) || near(cell, to))
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
    /// The match mode is not on the view, so no Hold objective is handed in
    /// (see the module head's residuals).
    fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
        let mut memory = StockMemory::from(&decode_memory(job.memory));
        let intents = self.decide(job.view, &mut memory, job.map, None, job.t_ms, job.lands_ms);
        ThinkResult {
            intents,
            memory: encode_memory(&Memory::from(memory)),
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
    /// The weapon to fire.
    fire: Option<SmolStr>,
}

/// One think's reading of the motor's two notices.
#[derive(Clone, Copy, Debug)]
struct Drive {
    /// The goal the motor said it reached, or `None`.
    reached: Option<(u16, u16)>,
}

impl Drive {
    /// Arrival at `cell`: the hull's own cell within [`ARRIVE_CELLS`], or the
    /// motor's word for that goal.
    fn arrived(self, here: Option<(u16, u16)>, cell: (u16, u16)) -> bool {
        here.is_some_and(|here| near(here, cell)) || self.reached == Some(cell)
    }
}

impl NativeStockMind {
    /// **`decide`, re-expressed** (see the module head): walk the memory one
    /// step over `view` and answer the intents, in emission order. `hold` is
    /// the match's Hold objective, which a think is never handed (the view
    /// does not carry the mode); `t_ms` is the think's lattice millisecond,
    /// the clock every memory stamp is written on, as the reflex writes its
    /// tick's; `lands_ms` is the think's landing millisecond
    /// ([`ThinkJob::lands_ms`]), the one its shot is resolved at.
    #[allow(clippy::too_many_lines)]
    fn decide(
        &self,
        view: &MindView,
        mem: &mut StockMemory,
        def: &ArenaDefinition,
        hold: Option<&HoldState>,
        t_ms: i64,
        lands_ms: i64,
    ) -> Vec<IntentKind> {
        let cfg = &self.cfg;
        let here = view.cell;
        let mut plan = Plan::default();

        // ---- what the motor and the rounds said since the last think ----
        //
        // The reflex learns its `no line` edge from the impact
        // (`Arena::note_no_line`) and its arrival and its `unreachable` edge
        // from the drive it runs itself; a mind reads all three off its notice
        // buffer (D-UM-8): a `blocked` or `cover` miss of its own round, and
        // the motor's `drive_arrived` / `drive_no_route` for the goal it
        // handed over.
        let mut arrived_word = false;
        let mut no_route = false;
        for notice in &view.notices {
            match notice {
                FrameNotice::Drive {
                    outcome: DriveOutcome::Arrived,
                } => arrived_word = true,
                FrameNotice::Drive {
                    outcome: DriveOutcome::NoRoute,
                } => no_route = true,
                FrameNotice::Miss { reason, .. }
                    if reason.as_str() == BLOCKED || reason.as_str() == COVER =>
                {
                    mem.no_line = true;
                }
                _ => {}
            }
        }
        let drive = Drive {
            reached: if arrived_word { mem.goal } else { None },
        };
        // What the motor holds now, as far as the mind knows: the goal it was
        // handed — which it keeps past an arrival, as the reflex's row does —
        // and nothing once it said no route reached it.
        let mut held = if no_route { None } else { mem.goal };
        // `motor.goal`, as `decide` writes it through the think.
        let mut goal = held;

        // ---- the `Disabled` edge, before anything else ----
        if view.wreck || (view.immobile && view.weapons.is_empty()) {
            *mem = StockMemory {
                state: ReflexState::Disabled,
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
            let live = if mem.ignore_order {
                None
            } else {
                view.order.as_ref()
            };
            // 16b cut E.2 (`B-41`): the order's own `Advance` cell.
            if live.is_some_and(|order| order.objective == MindObjective::Advance { to: failed }) {
                mem.ignore_order = true;
            }
            // 16b cut Z.2's review: a supporter's station, in `Patrol`.
            if live.is_some_and(|order| matches!(order.objective, MindObjective::Support { .. }))
                && mem.state == ReflexState::Patrol
            {
                mem.ignore_order = true;
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
        // set `ignore_order` and it stops obeying (the divergence
        // `order_end`); that is spent when the view carries no order.
        if view.order.is_none() {
            mem.ignore_order = false;
        }
        let health = health_permille(view);
        let dry = is_dry(view);
        // **The expiry's restoration** (`ReflexUnit::drop_to_doctrine`), read
        // off the view: a `Withdraw` that no live order holds and that neither
        // of its own edges would have taken — healthy and not dry — is one an
        // order put it in, and the order is over. It goes back to `Patrol` with
        // the target and the goal dropped, as the reflex's expiry does; a hurt
        // one keeps going home.
        if view.order.is_none()
            && mem.state == ReflexState::Withdraw
            && !dry
            && health >= cfg.withdraw_permille
        {
            mem.state = ReflexState::Patrol;
            mem.target = None;
            goal = None;
        }
        let mut order: Option<&FrameOrder> = if mem.ignore_order {
            None
        } else {
            view.order.as_ref()
        };
        let ends = match order.map(|order| order.objective) {
            // 16b cut AA.1 (`B-76`): an `Advance` a hull that cannot close can
            // never carry out. `immobile` is the view's `cannot_close`; the
            // hull authored with no speed is M4's residual.
            Some(MindObjective::Advance { .. }) if view.immobile => true,
            // 16b cut O.1: an `Advance` into the Hold square, arrived.
            Some(MindObjective::Advance { to })
                if hold.is_some_and(|hold| advance_reached_the_hold(hold, to, here)) =>
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
            mem.ignore_order = true;
            order = None;
        }
        let objective = order.map(|order| order.objective);

        // ---- the inert rule (16b cut O.3) ----
        //
        // A hull that cannot move and carries no weapon is inert. Over the view
        // that is `immobile` and no weapon rows — which is the `Disabled` edge's
        // own reading above, so it never reaches here: a hull AUTHORED with no
        // speed (the range's target) is the case the view cannot tell, M4's
        // residual.

        // ---- power ----
        match view.power {
            Power::Off => {
                // 16b cut D.3 (`B-34`): a cold tank starts up, wherever it is
                // going; a `Withdraw` stays one.
                if mem.state != ReflexState::Withdraw {
                    mem.state = ReflexState::Startup;
                }
                mem.goal = held;
                return vec![IntentKind::Startup];
            }
            Power::Starting => {
                if mem.state != ReflexState::Withdraw {
                    mem.state = ReflexState::Startup;
                }
                mem.goal = held;
                return Vec::new();
            }
            Power::Online => {
                if matches!(mem.state, ReflexState::Startup | ReflexState::Disabled) {
                    mem.state = ReflexState::Patrol;
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
            if matches!(mem.state, ReflexState::Approach | ReflexState::Engage) {
                mem.state = ReflexState::Patrol;
                goal = None;
            }
        }

        // ---- what the order does to the machine (16b cut N.2, Z.2) ----
        match objective {
            Some(MindObjective::Withdraw) => {
                if mem.state != ReflexState::Withdraw {
                    mem.state = ReflexState::Withdraw;
                    mem.target = None;
                    goal = None;
                    mem.last_plot = None;
                }
            }
            Some(
                MindObjective::Hold | MindObjective::Advance { .. } | MindObjective::Screen { .. },
            ) => {
                if matches!(mem.state, ReflexState::Approach | ReflexState::Engage) {
                    mem.state = ReflexState::Patrol;
                    goal = None;
                }
                mem.target = None;
            }
            Some(MindObjective::Engage { .. } | MindObjective::Support { .. }) | None => {}
        }

        // ---- the `Withdraw` edges (16b cut Q.7) ----
        if mem.state != ReflexState::Withdraw
            && (dry || (!mem.rallied && health < cfg.withdraw_permille))
        {
            mem.state = ReflexState::Withdraw;
            mem.target = None;
            goal = None;
            mem.last_plot = None;
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
                    ReflexState::Patrol | ReflexState::Approach | ReflexState::Engage
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
                ReflexState::Patrol | ReflexState::Approach | ReflexState::Engage
            )
        {
            match best_hostile(view, cfg, health, hint(order), |_| true) {
                Some(found) => {
                    if mem.target != Some(found) {
                        mem.target = Some(found);
                        goal = None;
                    }
                    if mem.state == ReflexState::Patrol {
                        mem.state = ReflexState::Approach;
                    }
                }
                None => {
                    mem.target = None;
                    if matches!(mem.state, ReflexState::Approach | ReflexState::Engage) {
                        mem.state = ReflexState::Patrol;
                        goal = None;
                    }
                }
            }
        }

        // ---- the `no line` edge (W's `blocked`/`cover` miss) ----
        if mem.no_line {
            mem.no_line = false;
            goal = None;
            if mem.state == ReflexState::Engage {
                mem.state = ReflexState::Approach;
            }
        }

        // ---- the state's own goal and orders ----
        let ranges = view_ranges(view, cfg, order);
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
                if matches!(mem.state, ReflexState::Approach | ReflexState::Engage) {
                    mem.state = ReflexState::Patrol;
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
                    engage_orders(view, row, order, &ranges, lands_ms, &mut plan);
                }
            }
            // (b) past the leash: back toward the station off its own plot of
            // the charge, firing nothing until it is back inside.
            Some(Some(plot)) if off_leash => {
                leashed = true;
                mem.state = ReflexState::Patrol;
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
                ReflexState::Startup | ReflexState::Disabled => {}
                ReflexState::Patrol => {
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
                                engage_orders(view, row, order, &ranges, lands_ms, &mut plan);
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
                                        station.is_some_and(|cell| drive.arrived(here, cell));
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
                            if !self.search_goal(t_ms, mem, &mut goal, here, drive) {
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
                ReflexState::Approach => {
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
                                mem.state = ReflexState::Engage;
                                goal = None;
                                if let Some(row) = row {
                                    engage_orders(view, row, order, &ranges, lands_ms, &mut plan);
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
                ReflexState::Engage => {
                    if let Some(target) = mem.target {
                        let row = contact(view, target);
                        let range = row.map_or(u64::MAX, row_range_cm);
                        if ranges.limit_cm == 0 || range > ranges.release_cm {
                            mem.state = ReflexState::Approach;
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
                                engage_orders(view, row, order, &ranges, lands_ms, &mut plan);
                            }
                        }
                    }
                }
                // 16b cut Q.7: home, the stand, the dwell, the rally.
                ReflexState::Withdraw => {
                    if goal.is_none() {
                        goal = here
                            .and_then(|cell| entry_cell(def, view.team, CostRow::Tracked, cell))
                            .or(here);
                    }
                    let arrived = view.immobile
                        || match (here, goal) {
                            (Some(_), Some(cell)) => drive.arrived(here, cell),
                            (_, None) => true,
                            _ => false,
                        };
                    let cornered = self.withdraw_stand(view, def, health, arrived, &ranges, order);
                    if let Some(row) = cornered {
                        stood_cell = row.cell;
                        mem.stand_until_ms = Some(t_ms.saturating_add(
                            REFLEX_STAND_TICKS.saturating_mul(i64::from(cfg.decide_ms)),
                        ));
                        mem.quiet_since_ms = None;
                        plan.stop = true;
                        plan.heading_mdeg = Some(row.bearing_deg.saturating_mul(1_000));
                        engage_orders(view, row, order, &ranges, lands_ms, &mut plan);
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
                                mem.state = ReflexState::Patrol;
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
        } else if matches!(mem.state, ReflexState::Approach | ReflexState::Engage)
            && let Some(cell) = mem
                .target
                .and_then(|target| contact(view, target))
                .and_then(|row| row.cell)
        {
            mem.last_plot = Some((cell, t_ms));
        }
        if mem.state != ReflexState::Patrol {
            mem.hunt = None;
            mem.unheld_since_ms = Some(t_ms);
        }

        // ---- the emission (the module head's order and its two rules) ----
        // (`Startup` is the power arm's, which answers it alone.)
        let mut intents: Vec<IntentKind> = Vec::new();
        // The helm half. A hull whose motion is disabled is not handed one:
        // every goal and heading would be refused `MotionDisabled`, as the
        // reflex's are, and the motor drops a goal it cannot drive anyway.
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
        if let Some(weapon) = plan.fire {
            intents.push(IntentKind::Fire { weapon });
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
        here: Option<(u16, u16)>,
        drive: Drive,
    ) -> bool {
        let Some((cell, at_ms)) = mem.last_plot else {
            return false;
        };
        if self.cfg.search_ms == 0 || t_ms.saturating_sub(at_ms) > i64::from(self.cfg.search_ms) {
            mem.last_plot = None;
            return false;
        }
        if drive.arrived(here, cell) {
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
            if drive.arrived(here, cell) {
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
            here.and_then(|cell| far_entry_cell(def, view.team, CostRow::Tracked, cell, &[]))
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
        let row = contact(view, best_hostile(view, &self.cfg, health, None, |_| true)?)?;
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
        let id = best_hostile(view, &self.cfg, health, hint(order), |row| {
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
    hold: &HoldState,
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
fn engage_orders(
    view: &MindView,
    row: &ViewContact,
    order: Option<&FrameOrder>,
    ranges: &Ranges,
    lands_ms: i64,
    plan: &mut Plan,
) {
    if view.weapons.is_empty() {
        return;
    }
    if view.lock != Some(row.id) {
        plan.lock = Some(row.id);
    }
    if view.power != Power::Online {
        return;
    }
    if order.is_some_and(|order| order.hold_fire) {
        return;
    }
    let range = row_range_cm(row);
    if range > ranges.release_cm {
        return;
    }
    // The milliseconds from the view's clock to the landing: a weapon whose
    // countdown ends inside them is ready when the shot is resolved.
    let to_landing = lands_ms.saturating_sub(view.t_ms).max(0);
    plan.fire = view
        .weapons
        .iter()
        .find(|weapon| {
            weapon.ready_in_ms <= to_landing
                && weapon.rounds > 0
                && weapon
                    .bands_cm
                    .last()
                    .is_some_and(|reach| u64::from(*reach) >= range)
                && !would_overheat(view, weapon)
        })
        .map(|weapon| weapon.id.clone());
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mush_core::ObjId;

    use super::*;
    use crate::arena::mind::{MIND_INTENTS_MAX, memory_check};
    use crate::arena::state::{
        Arena, ArenaTerrain, Controller, CustodyOut, CustodyUnit, MatchMode, SeenPose,
        ShadowContact, Team,
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
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the field arena: {error:?}"))
    }

    /// A line tank flown by a mind.
    fn minded(id: ObjId, team: Team, x: i64, y: i64) -> CustodyUnit {
        let mut unit = line_tank(id, team, pose(x, y));
        unit.controller = Controller::Mind;
        unit
    }

    /// `viewer` holds `target` under `id`, classified, plotted where it stands.
    fn hold(arena: &mut Arena, viewer: ObjId, target: ObjId, id: ContactId) {
        let Some(state) = arena.unit(target) else {
            panic!("a fielded target");
        };
        let class = Classification::Classified { team: state.team };
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
        )
    }

    /// **The emission order and its two rules** (the module head): kinds in
    /// `Startup, Goal, Heading, Speed, Lock, Fire` order, each at most once; a
    /// `Goal(Cell)` with no `Heading` or `Speed` beside it; never more than
    /// four (the module head's maximum: `Goal(Hold)`, `Heading`, `Lock`,
    /// `Fire`), which is under the brief's six and `MIND_INTENTS_MAX`.
    fn emission_holds(intents: &[IntentKind]) {
        let rank = |kind: &IntentKind| match kind {
            IntentKind::Startup => 0_u8,
            IntentKind::Goal(_) => 1,
            IntentKind::Heading { .. } => 2,
            IntentKind::Speed { .. } => 3,
            IntentKind::Lock { .. } => 4,
            IntentKind::Fire { .. } => 5,
            other => panic!("the stock mind emitted {other:?}"),
        };
        let ranks: Vec<u8> = intents.iter().map(rank).collect();
        assert!(
            ranks.windows(2).all(|pair| pair.first() < pair.get(1)),
            "out of order, or a kind twice: {intents:?}"
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
        assert!(intents.len() <= 4, "{intents:?}");
        assert!(intents.len() <= MIND_INTENTS_MAX);
    }

    /// **`StockMemory` is fourteen keys and round-trips** (17b cut M1a.6;
    /// D-UM-30): every field set — a target id with its top bit set among
    /// them — writes exactly [`STOCK_MEMORY_KEY_NAMES`], passes
    /// `memory_check`, reads back to itself, and `encode(decode(encode(m)))
    /// == encode(m)`; the default writes the same fourteen keys, its nulls
    /// included, and an empty memory reads as the default.
    #[test]
    fn m1a6_the_stock_memory_is_fourteen_keys_and_round_trips() {
        let full = StockMemory {
            state: ReflexState::Withdraw,
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
            ignore_order: true,
        };
        let mut keys: Vec<&str> = STOCK_MEMORY_KEY_NAMES.to_vec();
        keys.sort_unstable();
        for stock in [full.clone(), StockMemory::default()] {
            let memory = Memory::from(stock.clone());
            assert_eq!(
                memory.keys().map(SmolStr::as_str).collect::<Vec<&str>>(),
                keys
            );
            assert_eq!(memory_check(&memory), Ok(()));
            assert_eq!(StockMemory::from(&memory), stock);
            let bytes = encode_memory(&memory);
            assert_eq!(encode_memory(&decode_memory(&bytes)), bytes);
        }
        assert_eq!(STOCK_MEMORY_KEY_NAMES.len(), 14);
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
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
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
        let ranked = best_hostile_ranked(&view, cfg, 1_000, None, |_| true);
        assert_eq!(ranked.len(), 2);
        assert_eq!(
            ranked.first().map(|(score, _)| *score),
            ranked.get(1).map(|(score, _)| *score)
        );
        let best = |view: &MindView, prefer: Option<ContactId>| {
            best_hostile(view, cfg, 1_000, prefer, |_| true)
        };
        assert_eq!(best(&view, None), Some(ContactId(0x10)));
        assert_eq!(best(&view, Some(ContactId(0x30))), Some(ContactId(0x30)));
        assert_eq!(
            best_hostile(&view, cfg, 1_000, None, |row| row.id != ContactId(0x10)),
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
        let unknown = with(&|row| row.class = Classification::Unknown);
        assert_eq!(best(&unknown, None), None);
        let friends = with(&|row| row.class = Classification::Classified { team: Team::Red });
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
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
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
                best_hostile(view, cfg, health_permille(view), None, |_| true),
                Some(ContactId(0x77))
            );
        }
        assert_eq!(
            view_ranges(&before, cfg, None),
            view_ranges(&after, cfg, None)
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
    /// six-intent cap**: a cold hull answers `Startup` alone; a patrol hands
    /// the motor a cell and nothing else, and the same think again hands it
    /// nothing (the goal did not change); a hostile at 200 m with a line is
    /// engaged — the lock and the shot on the think that finds it — and the
    /// next think stands: `Goal(Hold)`, the facing `Heading`, then the lock and
    /// the shot, in that order.
    #[test]
    fn m1a6_the_emission_order_and_its_two_rules() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        let mut memory = StockMemory::default();
        let cold = think(&arena, &view_of(&arena, RED), &mut memory, 0);
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
        assert_eq!(again, Vec::new(), "the goal did not change");
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut fight = StockMemory::default();
        let found = think(&arena, &view_of(&arena, RED), &mut fight, 1_000);
        emission_holds(&found);
        assert_eq!(fight.state, ReflexState::Engage);
        assert!(
            matches!(
                found.as_slice(),
                [IntentKind::Lock { contact }, IntentKind::Fire { .. }]
                    if *contact == ContactId(0x77)
            ),
            "{found:?}"
        );
        let stands = think(&arena, &view_of(&arena, RED), &mut fight, 2_000);
        emission_holds(&stands);
        assert!(
            matches!(
                stands.as_slice(),
                [
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { mdeg: 0 },
                    IntentKind::Lock { .. },
                    IntentKind::Fire { .. }
                ]
            ),
            "{stands:?}"
        );
        assert_eq!(fight.goal, None, "a stand holds no goal");
    }

    /// **The `drive_no_route` port** (departure (i)): a view carrying the
    /// notice for the memory's goal performs the reflex's `unreachable` edge a
    /// think late — the search's plot and the hunt's cell on that goal given
    /// up (the hunt's clock restarted), an `Advance` to it ended by
    /// `ignore_order` — and hands the motor the fallback waypoint
    /// ([`fallback_waypoint`]: from (3, 4) the nearest node, west at (1, 4))
    /// as the next `Goal(Cell)`.
    #[test]
    fn m1a6_a_drive_no_route_notice_performs_the_give_ups_and_the_fallback() {
        let mut arena = field(vec![minded(RED, Team::Red, 3_500, 4_500)]);
        online(&mut arena, RED);
        let failed = (6, 4);
        let mut view = view_of(&arena, RED);
        view.notices = vec![FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        }];
        let start = StockMemory {
            state: ReflexState::Patrol,
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
        assert!(!memory.ignore_order);
        // The same notice under an `Advance` to the failed cell ends the order.
        view.order = Some(FrameOrder {
            squad: SmolStr::new("red-1"),
            objective: MindObjective::Advance { to: failed },
            hold_fire: false,
            max_range_band: None,
            expires_ms: 60_000,
        });
        let mut ordered = start;
        let _ = think(&arena, &view, &mut ordered, 10_000);
        assert!(ordered.ignore_order, "the order is over for this hull");
        // No notice, no give-up: the hull drives on to the goal it holds.
        let mut quiet = StockMemory {
            state: ReflexState::Patrol,
            goal: Some((1, 4)),
            waypoint: Some(0),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        assert_eq!(
            think(&arena, &view_of(&arena, RED), &mut quiet, 10_000),
            Vec::new()
        );
    }

    /// **The `drive_arrived` arrival** (departure (d)): a hunt whose cell the
    /// motor fanned the hull off — the hull nowhere near the named cell — is
    /// over when the motor says it arrived: the hunt is spent, its clock
    /// restarted, the walk forgotten and resumed from the nearest node, which
    /// is the next goal. Without the notice the hull drives on.
    #[test]
    fn m1a6_a_drive_arrived_notice_is_arrival_wherever_the_fan_landed() {
        let mut arena = field(vec![minded(RED, Team::Red, 3_500, 4_500)]);
        online(&mut arena, RED);
        let named = (50, 40);
        let start = StockMemory {
            state: ReflexState::Patrol,
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
            Vec::new(),
            "driving on"
        );
        assert_eq!(on.hunt, Some(named));
        view.notices = vec![FrameNotice::Drive {
            outcome: DriveOutcome::Arrived,
        }];
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
                answers.push((job.view.clone(), result.clone()));
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
        let mut red = scout(RED, Team::Red, pose(20_500, 20_500));
        red.weapons.clear();
        red.controller = Controller::Mind;
        let mut arena = field(vec![
            red,
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
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
            answers.iter().any(|(view, _)| view
                .contacts
                .iter()
                .any(|row| matches!(row.class, Classification::Classified { team: Team::Blue }))),
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
            assert!(
                !result
                    .intents
                    .iter()
                    .any(|kind| matches!(kind, IntentKind::Lock { .. } | IntentKind::Fire { .. })),
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
            squad: SmolStr::new("red-1"),
            objective,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 60_000,
        }
    }

    /// The Hold objective on the spot `(30, 20)`, a three-cell square.
    fn spot() -> HoldState {
        HoldState {
            spot: (30, 20),
            letter: Some('A'),
            radius_cells: 3,
            hold_ms: 60_000,
            held_by: None,
        }
    }

    /// One think of the stock mind handed the match's Hold objective.
    fn think_holding(
        arena: &Arena,
        view: &MindView,
        memory: &mut StockMemory,
        hold: &HoldState,
        t_ms: i64,
    ) -> Vec<IntentKind> {
        NativeStockMind::with(&arena.arena_cfg.reflex).decide(
            view,
            memory,
            &arena.definition,
            Some(hold),
            t_ms,
            t_ms.saturating_add(arena.substep_len_ms()),
        )
    }

    /// A patrolling memory with nothing seen and the hunt's clock just set.
    fn patrolling(t_ms: i64) -> StockMemory {
        StockMemory {
            state: ReflexState::Patrol,
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
        let mut arena = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
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
        refused.notices = vec![FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        }];
        let retry = {
            let cfg = &arena.arena_cfg.reflex;
            2_000 + i64::from(cfg.hunt_after_ms.max(cfg.repath_ms))
        };
        let fell_back = think_holding(&arena, &refused, &mut memory, &hold, 2_000);
        emission_holds(&fell_back);
        assert_eq!(memory.hold_retry_ms, Some(retry));
        assert!(
            matches!(fell_back.as_slice(), [IntentKind::Goal(MotorGoal::Cell { x, y })]
                if (*x, *y) != hold.spot),
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
        let mut there = field(vec![minded(RED, Team::Red, 31_500, 20_500)]);
        online(&mut there, RED);
        let mut parked = StockMemory {
            goal: Some(hold.spot),
            ..patrolling(0)
        };
        let stop = think_holding(&there, &view_of(&there, RED), &mut parked, &hold, 3_000);
        assert_eq!(stop, vec![IntentKind::Goal(MotorGoal::Hold)]);
        assert_eq!(parked.goal, None);
        assert_eq!(parked.state, ReflexState::Patrol);
    }

    /// **An `Advance` into the Hold square ends on arrival** (16b cut O.1's
    /// order end, as a mind performs it: `ignore_order`), and the hull is
    /// parked by the Hold arm on the same think; with no Hold objective the
    /// same `Advance` stands and is driven. **And `Approach` inside the square
    /// does not leave it** (16b cut O.1): a target past the hull's reach is
    /// approached by holding the cell it stands on, not by the stand-off cell
    /// out of the square.
    #[test]
    fn m1a6_an_advance_into_the_square_ends_and_an_approach_inside_it_stays() {
        let hold = spot();
        let mut arena = field(vec![
            minded(RED, Team::Red, 31_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(31_500, 55_500)),
        ]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Advance { to: (32, 20) }));
        let mut memory = patrolling(0);
        let ended = think_holding(&arena, &view, &mut memory, &hold, 1_000);
        assert!(memory.ignore_order, "the Advance is over for this hull");
        assert_eq!(ended, vec![IntentKind::Goal(MotorGoal::Hold)]);
        // With no Hold objective the Advance stands, and is driven.
        let mut skirmish = patrolling(0);
        let driven = think(&arena, &view, &mut skirmish, 1_000);
        assert!(!skirmish.ignore_order);
        assert_eq!(
            driven,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 32, y: 20 })]
        );
        // `Approach`, inside the square, on a row past its reach.
        hold_contact(&mut arena, RED, BLUE, ContactId(0x55));
        let mut view = view_of(&arena, RED);
        let cfg = &arena.arena_cfg.reflex;
        let ranges = view_ranges(&view, cfg, None);
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
            state: ReflexState::Approach,
            target: Some(ContactId(0x55)),
            ..patrolling(0)
        };
        let inside = think_holding(&arena, &view, &mut closing, &hold, 1_000);
        assert_eq!(closing.state, ReflexState::Approach);
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
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
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
        assert_eq!(memory.state, ReflexState::Patrol);
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
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { mdeg: 0 },
                    IntentKind::Lock { contact },
                    IntentKind::Fire { .. }
                ] if *contact == ContactId(0x11)
            ),
            "{inside:?}"
        );
        assert_eq!(
            memory.state,
            ReflexState::Patrol,
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
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(MATE, Team::Red, pose(20_500, 50_500)),
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
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. },
                    IntentKind::Lock { .. },
                    IntentKind::Fire { .. }
                ]
            ),
            "{stood:?}"
        );
        assert_eq!(blind.target, None);
        assert_eq!(blind.state, ReflexState::Patrol);
        // (b) Off the leash: 30 cells from the charge's plot.
        hold(&mut arena, RED, MATE, ContactId(0x99));
        let mut leashed_view = view_of(&arena, RED);
        leashed_view.order = Some(order_of(MindObjective::Support {
            of: Some(ContactId(0x99)),
        }));
        let mut leashed = StockMemory {
            state: ReflexState::Engage,
            target: Some(ContactId(0x11)),
            ..patrolling(0)
        };
        let back = think(&arena, &leashed_view, &mut leashed, 1_000);
        emission_holds(&back);
        assert!(
            matches!(back.as_slice(), [IntentKind::Goal(MotorGoal::Cell { y, .. })] if *y > 20),
            "toward the station, south, firing nothing: {back:?}"
        );
        assert_eq!(leashed.state, ReflexState::Patrol);
        assert_eq!(leashed.target, None);
        // (c) Inside its station: parked.
        let mut close = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(MATE, Team::Red, pose(20_500, 23_500)),
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
        let mut arena = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
        online(&mut arena, RED);
        let view = view_of(&arena, RED);
        let withdrawing = StockMemory {
            state: ReflexState::Withdraw,
            target: Some(ContactId(0x11)),
            ..StockMemory::default()
        };
        let mut expired = withdrawing.clone();
        let _ = think(&arena, &view, &mut expired, 1_000);
        assert_eq!(expired.state, ReflexState::Patrol);
        assert_eq!(expired.target, None);
        let mut ordered_view = view.clone();
        ordered_view.order = Some(order_of(MindObjective::Withdraw));
        let mut ordered = withdrawing.clone();
        let _ = think(&arena, &ordered_view, &mut ordered, 1_000);
        assert_eq!(ordered.state, ReflexState::Withdraw);
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
        assert_eq!(hurt.state, ReflexState::Withdraw);
    }

    /// **The `no line` edge off a miss notice** (W's `blocked`/`cover`; the
    /// divergence `no_line`): an `Engage` holding its band stands and shoots;
    /// the same think with a `blocked` miss of its own round in the view drops
    /// to `Approach`, throws the stand away and drives for the stand-off cell
    /// — no lock, no shot — and the flag is spent.
    #[test]
    fn m1a6_a_blocked_miss_notice_turns_engage_into_approach() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut view = view_of(&arena, RED);
        let ranges = view_ranges(&view, &arena.arena_cfg.reflex, None);
        // Between the two edges: `Engage` keeps it, `Approach` would close.
        let between_m = (ranges.engage_cm + ranges.release_cm) / 200;
        for row in &mut view.contacts {
            row.range_m = between_m;
        }
        let engaged = StockMemory {
            state: ReflexState::Engage,
            target: Some(ContactId(0x77)),
            ..patrolling(0)
        };
        let mut standing = engaged.clone();
        let stand = think(&arena, &view, &mut standing, 1_000);
        assert_eq!(standing.state, ReflexState::Engage);
        assert!(
            stand.contains(&IntentKind::Goal(MotorGoal::Hold)),
            "{stand:?}"
        );
        view.notices = vec![FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new(BLOCKED),
        }];
        let mut blocked = engaged;
        let closed = think(&arena, &view, &mut blocked, 1_000);
        emission_holds(&closed);
        assert_eq!(blocked.state, ReflexState::Approach);
        assert!(!blocked.no_line, "spent");
        assert!(
            matches!(
                closed.as_slice(),
                [IntentKind::Goal(MotorGoal::Cell { .. })]
            ),
            "{closed:?}"
        );
    }
}
