//! **The reflex controller** (16b cut N.1; doc 16 §6 "AI", §3.4, §3.5, §3.8
//! "Disconnects"; 16b §8's cut N row; D-16-16).
//!
//! A deterministic state machine per AI unit:
//!
//! ```text
//! Startup → Patrol → Approach → Engage
//!              ↑         ↓         ↓
//!              └─────────┴──→ Withdraw → (holds)
//!   any state, on a wreck or a disabling ──→ Disabled
//! ```
//!
//! It drives the harness's field of 300; the AI fill of a lobby that boards fewer
//! pilots than there are tanks, behind `[tacsim.match].ai_fill` (off by default —
//! `crate::config::MatchConfig::ai_fill` says why); and — replacing K4's idle
//! substitution — the tank of a seat released past the linkdead grace, which it
//! takes over in `Withdraw` (doc 16 §3.8's amendment, doc 07 §1.1's fallback
//! controller).
//!
//! # It can do nothing a pilot cannot
//!
//! Every order it gives is an [`Intent`] built with the unit's own epoch and put
//! through [`apply_intent`] — the same function pulse phase 2 puts a pilot's verbs
//! through — and every discrete action it stages is resolved by the same
//! `resolve_startup`, `Arena::resolve_lock` and `Arena::resolve_fire` that phase 4
//! calls. So the wreck gate, the channel gate, the epoch gate, the linkdead gate,
//! the queue bound, `PowerNotOnline`, `Cycling` and `Empty` all apply to the
//! machine exactly as they apply to a player, and a controller bug cannot invent a
//! capability. What the machine adds on top is only *politeness*: it reads
//! `weapon_ready`, `weapon_ammo`, `power` and `lock` before it stages a `Fire`, so
//! it does not feed a refusal into the resolver every tick.
//!
//! # Why the tick is a due event and not pulse phase 4
//!
//! **This is a deviation from 16b §8's cut N row and doc 16 §6, both of which say
//! phase 4, and it is forced by the same cut's other requirement.** A controller
//! run once per *pulse* runs once per 5,000 ms at `Quiet` and ten times in the
//! same arena interval at `Engaged`: the units would steer, close and shoot at a
//! rate set by the tempo. Doc 16 §3.1's D-16-2 is that *the tempo decides only how
//! often the arena steps and how its output is batched, never an outcome*, and the
//! cut N row asks for the cross-tempo property — one event list and one hash
//! sequence at 5,000, 1,000 and 500 ms — **extended with two reflex units**. Those
//! two cannot both be true of a per-pulse controller.
//!
//! So the tick is a [`DueKind::ReflexTick`], scheduled every
//! `arena.reflex.decide_ms` of arena time and staggered by unit id, applied inside
//! the substep whose boundary its timestamp fell in — exactly the shape doc 16
//! §3.6's sensor pass and §3.5's scheduled impact already have, and exactly what
//! makes those two rate-independent. The intents it stages are resolved **at that
//! timestamp**, in the same event, rather than left in the queue for a phase 4
//! whose `now_ms` is the pulse's end: a `fired` line at the pulse boundary would
//! carry a different arena millisecond at every cadence, which is the same
//! divergence read once more.
//!
//! The one thing phase 4 keeps is the hash discipline: a substep that runs reflex
//! work takes **one** [`crate::arena::hash::state_hash`] before its due-event loop
//! and every action inside it carries that one hash, the same way phase 4 takes
//! one for the whole phase ([`crate::arena::state::Arena::action_hash`]).
//!
//! # No draw
//!
//! Doc 16 §3.5 pins the draw-label set and a contract test gates it. Nothing here
//! draws: every choice is a total order over integers, and the tie-breaker is the
//! unit id (`Arena::units` is kept in that order, so "the first" is the lowest id).
//!
//! # The bounds
//!
//! Path searches are the only unbounded-looking thing a controller does, and they
//! are bounded three ways: [`crate::arena::path::path`] expands at most
//! `reflex.path_max_cells` nodes (16b §9's raised `[atlas].path_max_cells`); one
//! unit searches at most once per `reflex.repath_ms`; and the whole field searches
//! at most `reflex.max_path_queries` times per `reflex.decide_ms` **window of
//! arena time**, a unit refused by that budget keeping the path it has. The window
//! is arena time and not a pulse for D-16-2's reason once more;
//! `[tacsim.harness].max_path_queries_per_pulse` stays what its own block says it
//! is, a threshold the harness asserts against [`ReflexCounters::path_queries`].

use std::collections::BTreeMap;

use mush_atlas::Cover;
use mush_core::ObjId;
use smol_str::SmolStr;

use crate::arena::intents::{Intent, IntentKind, IntentOutcome, apply_intent, resolve_startup};
use crate::arena::ledger::EventKind;
use crate::arena::motion::BlockedCause;
use crate::arena::path::{PathScratch, path_with, shoulder_refuses};
use crate::arena::perception::bearing_of;
use crate::arena::resolution::facet_band_seen;
use crate::arena::slope;
use crate::arena::state::{
    Arena, ArenaTerrain, Classification, ContactId, Controller, DueKind, Power, SeenPose, UnitState,
};
use crate::config::ReflexConfig;
use crate::deliberation::order::{Objective, Order, StandingOrder};

/// How near a goal cell counts as arrived, in cells.
///
/// One, because occupancy is one unit per cell (doc 16 §3.4 step 5): a controller
/// that insisted on standing *in* its goal would sit against a teammate for ever.
pub const ARRIVE_CELLS: u16 = 1;

/// The most cells the `Withdraw` goal search reads out of an entry zone before it
/// settles for the clamped corner.
///
/// An entry zone is sixteen cells deep and a window tall (the shipped arena's is
/// 16 by 256), and this search runs once per `repath_ms` per withdrawing unit, so
/// it is bounded for the same reason everything else here is.
pub const MAX_ENTRY_SCAN: usize = 1_024;

/// **The most cells one unit remembers being stopped at** (16b cut Q.7), oldest
/// dropped.
///
/// Eight, because the list is a memory of *this* stop and the two or three before
/// it — a tank that is being herded round a wreck needs the wreck's cell and the
/// cells it tried instead, and nothing older than that. It is also what bounds
/// the scan [`crate::arena::path::path_with`] pays per neighbour.
pub const REFLEX_AVOID_MAX: usize = 8;

/// **How long a remembered stop lasts**, arena milliseconds (16b cut Q.7).
///
/// Entries **age**; they are not cleared when the goal changes. `Approach`
/// recomputes its goal from the contact's plot at every tick, so a
/// clear-on-goal-change would wipe the list about once a second — in exactly the
/// case the owner hit, a tank chasing a moving target past a wreck. Thirty
/// seconds is long enough to get round an obstacle and short enough that a wreck
/// that has been salvaged, or a tank that has driven off, stops being a hole in
/// the map.
pub const REFLEX_AVOID_TTL_MS: i64 = 30_000;

/// **How many decide cycles a cornered `Withdraw` stands and fights** (16b cut
/// Q.7) before it asks again whether it still has a line.
///
/// Three, because the live line a `Withdraw` is refusing to turn its back on
/// flickers: a treeline between two tanks breaks and remakes the trace several
/// times a second of arena time, and a unit that resumed its run on the first
/// broken tick would spend the next one turning round again.
pub const REFLEX_STAND_TICKS: i64 = 3;

/// The exposure, per mille, of each facet band a candidate may present — the
/// `weight_facet` term of doc 16 §6's utility score.
///
/// Lower is a better target: a tank showing its rear is worth closing on and one
/// showing its front is not. The numbers are doctrine and nothing reads them but
/// the score.
const EXPOSURE_FRONT: u32 = 1_000;
/// A flank.
const EXPOSURE_FLANK: u32 = 600;
/// The rear.
const EXPOSURE_REAR: u32 = 200;

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

/// **Where one AI unit is in doc 16 §6's machine.**
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReflexState {
    /// Cold: the only order it gives is `STARTUP`.
    #[default]
    Startup,
    /// Walking the arena's waypoint graph.
    Patrol,
    /// Closing on the hostile the utility score picked.
    Approach,
    /// Holding the band, locked, firing when the breech is free.
    Engage,
    /// Below the health threshold and making for its own entry zone.
    Withdraw,
    /// A wreck, or disabled: it issues nothing at all.
    Disabled,
}

impl ReflexState {
    /// Every state, in code order — what a distribution is reported over.
    pub const ALL: [ReflexState; 6] = [
        ReflexState::Startup,
        ReflexState::Patrol,
        ReflexState::Approach,
        ReflexState::Engage,
        ReflexState::Withdraw,
        ReflexState::Disabled,
    ];

    /// The wire word.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            ReflexState::Startup => "startup",
            ReflexState::Patrol => "patrol",
            ReflexState::Approach => "approach",
            ReflexState::Engage => "engage",
            ReflexState::Withdraw => "withdraw",
            ReflexState::Disabled => "disabled",
        }
    }

    /// The byte [`crate::arena::hash::state_hash`] folds in.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            ReflexState::Startup => 0,
            ReflexState::Patrol => 1,
            ReflexState::Approach => 2,
            ReflexState::Engage => 3,
            ReflexState::Withdraw => 4,
            ReflexState::Disabled => 5,
        }
    }
}

/// **What the controller remembers about one unit.**
///
/// Every field is a fact the machine wrote and can re-derive from nothing else:
/// which state it is in, where it is going, the route it is following and how far
/// along it, which hostile it settled on, and when it last searched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReflexUnit {
    /// Where it is in the machine.
    pub state: ReflexState,
    /// The cell it is making for.
    pub goal: Option<(u16, u16)>,
    /// The waypoint node `Patrol` is walking toward.
    pub waypoint: Option<u16>,
    /// The node it came from, so a patrol does not turn round at every node.
    pub from_waypoint: Option<u16>,
    /// The route to [`ReflexUnit::goal`], nearest cell first; consumed as the unit
    /// crosses each one.
    pub route: Vec<(u16, u16)>,
    /// The arena millisecond of its last path search, whatever the search found —
    /// a failure costs a search, or an unreachable goal would be rediscovered
    /// every tick.
    pub searched_ms: Option<i64>,
    /// The hostile it settled on, and the contact it holds it under.
    pub target: Option<(ContactId, ObjId)>,
    /// A shot of this unit's was refused the line (W's `blocked` or `cover` miss):
    /// the `no line` edge, cleared by the tick that re-plans on it.
    pub no_line: bool,
    /// **The cells this unit was stopped at by something standing in them**, with
    /// the arena millisecond of the stop (16b cut Q.7; doc 16 §3.4 step 5).
    ///
    /// Written by [`Arena::note_blocked`] off the `Blocked` **ledger event** —
    /// the one place the cell and the cause are both in hand — and read by the
    /// path search, which treats every listed cell as impassable. At most
    /// [`REFLEX_AVOID_MAX`] entries, oldest dropped; a repeat of a listed cell
    /// refreshes its stamp; an entry older than [`REFLEX_AVOID_TTL_MS`] is
    /// dropped at the top of a tick.
    ///
    /// It is **not** occupancy and it cannot be used as one: the unit knows it
    /// was refused a cell, and never what refused it. The occupant may be a
    /// platform it has not detected (doc 16 §3.6).
    pub avoid: Vec<((u16, u16), i64)>,
    /// **While this is in the future, a cornered `Withdraw` stands** (16b cut
    /// Q.7): stopped, turned onto the hostile's plot, firing when the line is
    /// there. Set to `t_ms + REFLEX_STAND_TICKS × decide_ms` by the tick that
    /// found the line, and cleared by the first tick past it with no line.
    pub stand_until_ms: Option<i64>,
    /// **When this `Withdraw` last had nothing hostile on its plot board** (16b
    /// cut Q.7), arena milliseconds; `None` while it holds one, or while it is
    /// still on its way home. [`crate::config::ReflexConfig::rally_ms`] of it is
    /// what returns the unit to `Patrol`.
    pub quiet_since_ms: Option<i64>,
    /// **This unit has already come back into the fight once** (16b cut Q.7).
    ///
    /// The health edge is not taken twice: a tank that withdrew, waited out its
    /// quiet and rallied fights to the end. A commander's `Objective::Withdraw`
    /// still overrides it, and so does having nothing left to fire.
    pub rallied: bool,
    /// **The squad order it is under** (16b cut N.2; doc 16 §6 "Deliberation"),
    /// with the arena milliseconds it was accepted at and runs out at.
    ///
    /// `None` is doctrine, which is what doc 16 §6 calls *the fallback when nothing
    /// arrives*: not idle, not a hold — the machine below, exactly as it ran before
    /// an order arrived and exactly as it runs after one expires.
    pub order: Option<StandingOrder>,
}

// ---------------------------------------------------------------------------
// Counters
// ---------------------------------------------------------------------------

/// **What the controller cost one pulse** (16b cut N.1), the shape
/// [`crate::arena::perception::PerceptionCounters`] has and for the same reason:
/// the harness asserts a bound against the worst pulse of a run and the report
/// carries the totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReflexCounters {
    /// Ticks applied.
    pub ticks: u32,
    /// Path searches run — what `harness.max_path_queries_per_pulse` bounds.
    pub path_queries: u32,
    /// Searches refused by the per-window budget or by `repath_ms`.
    pub path_deferred: u32,
    /// Nodes those searches expanded.
    pub path_cells: u64,
    /// Searches that found no route, ran out of budget or were off the ground —
    /// the `unreachable` edge.
    pub path_failures: u32,
    /// Line-of-sight traces the controller took (`Approach`'s and `Engage`'s
    /// check that a shot has a line at all).
    pub traces: u32,
    /// Intents staged.
    pub intents: u32,
    /// `FIRE`s accepted.
    pub shots: u32,
    /// **Standing orders a tick acted on** (16b cut N.2) — ticks that read a live
    /// order, not orders accepted.
    pub ordered: u32,
    /// **Orders that ran out** (16b cut N.2), one per unit dropped to doctrine.
    pub orders_expired: u32,
    /// Ticks that ended in each state, in [`ReflexState::ALL`] order.
    pub in_state: [u32; 6],
}

impl ReflexCounters {
    /// Add another pulse's counts — the harness's running total.
    pub fn absorb(&mut self, other: &ReflexCounters) {
        self.ticks = self.ticks.saturating_add(other.ticks);
        self.path_queries = self.path_queries.saturating_add(other.path_queries);
        self.path_deferred = self.path_deferred.saturating_add(other.path_deferred);
        self.path_cells = self.path_cells.saturating_add(other.path_cells);
        self.path_failures = self.path_failures.saturating_add(other.path_failures);
        self.traces = self.traces.saturating_add(other.traces);
        self.intents = self.intents.saturating_add(other.intents);
        self.shots = self.shots.saturating_add(other.shots);
        self.ordered = self.ordered.saturating_add(other.ordered);
        self.orders_expired = self.orders_expired.saturating_add(other.orders_expired);
        for (mine, theirs) in self.in_state.iter_mut().zip(other.in_state.iter()) {
            *mine = mine.saturating_add(*theirs);
        }
    }
}

// ---------------------------------------------------------------------------
// The memory
// ---------------------------------------------------------------------------

/// **Everything the controller holds for the life of one match**: the per-unit
/// machines, the A\* scratch, the frozen doctrine and the path budget's window.
///
/// It lives on [`Arena`] rather than on `UnitState` because the scratch is one
/// half-megabyte buffer for the whole match rather than one per unit, and because
/// a unit under a pilot carries no machine at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReflexMemory {
    units: BTreeMap<ObjId, ReflexUnit>,
    scratch: PathScratch,
    cfg: ReflexConfig,
    /// **The cheapest cell on the window**, for the A\* heuristic's scale. Read
    /// once at check-out ([`crate::arena::path::min_move_cost`] is a scan of the
    /// whole window) and handed to every search, because the terrain is immutable
    /// for the life of a match.
    min_move_cost: u32,
    /// Which `decide_ms` window of arena time the budget below belongs to.
    window: i64,
    /// Searches already spent in that window.
    spent: u32,
}

impl ReflexMemory {
    /// An empty memory with this match's doctrine frozen in, exactly as the two
    /// configuration blocks are frozen into [`Arena`].
    #[must_use]
    pub fn new(cfg: &ReflexConfig, min_move_cost: u32) -> ReflexMemory {
        ReflexMemory {
            units: BTreeMap::new(),
            scratch: PathScratch::default(),
            cfg: cfg.clone(),
            min_move_cost: min_move_cost.max(1),
            window: i64::MIN,
            spent: 0,
        }
    }

    /// The doctrine this match is running on.
    #[must_use]
    pub fn config(&self) -> &ReflexConfig {
        &self.cfg
    }

    /// What the controller remembers about one unit, if it has ever ticked.
    #[must_use]
    pub fn unit(&self, unit: ObjId) -> Option<&ReflexUnit> {
        self.units.get(&unit)
    }

    /// Every machine, in unit-id order.
    pub fn all(&self) -> impl Iterator<Item = (ObjId, &ReflexUnit)> {
        self.units.iter().map(|(unit, row)| (*unit, row))
    }

    /// How many machines are in each state, in [`ReflexState::ALL`] order — the
    /// distribution the harness reports.
    #[must_use]
    pub fn distribution(&self) -> [u32; 6] {
        let mut out = [0_u32; 6];
        for row in self.units.values() {
            if let Some(slot) = out.get_mut(usize::from(row.state.code())) {
                *slot = slot.saturating_add(1);
            }
        }
        out
    }

    /// Whether any machine is in `Approach` — doc 16 §3.1's `Alert` trigger *a
    /// controller has entered `Approach`*.
    #[must_use]
    pub fn any_approaching(&self) -> bool {
        self.units
            .values()
            .any(|row| row.state == ReflexState::Approach)
    }

    /// **Hand a unit to the controller in a named state** — the check-out's
    /// `Reflex` units (`Startup`) and doc 16 §3.8's released seat (`Withdraw`).
    ///
    /// Idempotent for a unit already under the controller: a second `Control` of
    /// the same kind does not restart a machine halfway through a patrol.
    pub fn adopt(&mut self, unit: ObjId, state: ReflexState) {
        let row = self.units.entry(unit).or_default();
        if row.state != state {
            row.state = state;
            row.goal = None;
            row.route.clear();
            row.searched_ms = None;
            // **16b cut Q.7**: a machine pointed at a new state starts with a
            // clean memory of where it was stopped and whether it has already
            // rallied. A seat released into `Withdraw` is the case that matters:
            // it becomes an AI teammate that can come back, not one that has
            // already used its one return.
            row.avoid.clear();
            row.stand_until_ms = None;
            row.quiet_since_ms = None;
            row.rallied = false;
        }
    }

    /// Forget a unit's machine — a seat reclaimed, or a unit that has left.
    pub fn release(&mut self, unit: ObjId) {
        self.units.remove(&unit);
    }

    /// **Write a squad's standing order onto one unit's machine** (16b cut N.2),
    /// latest-wins.
    ///
    /// A unit with no machine gets one, in [`ReflexState::Startup`]: an order may
    /// arrive for a tank the controller has not ticked yet, and refusing it would
    /// make the first order of a match the one that never lands.
    pub fn set_order(&mut self, unit: ObjId, order: StandingOrder) {
        self.units.entry(unit).or_default().order = Some(order);
    }

    /// The order a unit is under, if any.
    #[must_use]
    pub fn order(&self, unit: ObjId) -> Option<&StandingOrder> {
        self.units.get(&unit).and_then(|row| row.order.as_ref())
    }

    /// How many machines are under a live order at `t_ms` — what the harness
    /// reports and a test asserts.
    #[must_use]
    pub fn under_order(&self, t_ms: i64) -> u32 {
        u32::try_from(
            self.units
                .values()
                .filter(|row| row.order.as_ref().is_some_and(|on| !on.expired_at(t_ms)))
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// Whether this tick may spend a path search, against the per-window budget.
    fn may_search(&mut self, t_ms: i64) -> bool {
        let window_ms = i64::from(self.cfg.decide_ms.max(1));
        let window = t_ms.div_euclid(window_ms);
        if window != self.window {
            self.window = window;
            self.spent = 0;
        }
        if self.spent >= self.cfg.max_path_queries {
            return false;
        }
        self.spent = self.spent.saturating_add(1);
        true
    }
}

// ---------------------------------------------------------------------------
// The plan one tick produces
// ---------------------------------------------------------------------------

/// The orders one tick decided on, before any of them has touched the arena.
///
/// A plain record so that the deciding half of a tick can borrow the arena
/// immutably and the ordering half can borrow it mutably, which is also what keeps
/// "decide from the world the substep committed" honest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ReflexPlan {
    startup: bool,
    heading_mdeg: Option<u32>,
    speed_cms: Option<i32>,
    lock: Option<ContactId>,
    fire: Option<SmolStr>,
    /// **16b cut N.2**: this tick found the standing order expired and dropped the
    /// unit to doctrine — the squad it was written for and the objective it was
    /// carrying, for the one `order_expired` line the tick journals.
    ///
    /// It rides the plan because `decide` holds `&self` and the ledger needs
    /// `&mut self`, which is the same split every other order on the plan has.
    expired: Option<(crate::deliberation::squad::SquadId, &'static str)>,
    /// **16b cut N.2**: the tick read a live order.
    ordered: bool,
}

// ---------------------------------------------------------------------------
// The tick
// ---------------------------------------------------------------------------

impl Arena {
    /// **The first [`DueKind::ReflexTick`] of every unit the check-out handed to
    /// the controller**, at `id % decide_ms` of arena time (doc 16 §6 read the way
    /// §3.6's pass is: *staggered by unit id*).
    ///
    /// Only the controller's units get one. A tank with a pilot in it schedules
    /// nothing at all — a match of twelve pilots costs exactly what it cost before
    /// this cut — and [`Arena::set_controller`] is what puts a unit on the cadence
    /// when its seat is released mid-match.
    pub fn schedule_first_reflex_ticks(&mut self) {
        let reflex: Vec<ObjId> = self
            .units
            .iter()
            .filter(|unit| unit.controller == Controller::Reflex)
            .map(|unit| unit.id)
            .collect();
        for id in reflex {
            self.reflex.adopt(id, ReflexState::Startup);
            self.schedule_reflex_tick(id);
        }
    }

    /// Put one unit's next tick on the stagger grid: the next multiple of
    /// `decide_ms` offset by `id % decide_ms` that is strictly after `now_ms`.
    ///
    /// The grid is a function of arena time and the unit id alone, so a unit
    /// adopted halfway through a match lands on the same cadence as one adopted at
    /// check-out and the field stays spread.
    fn schedule_reflex_tick(&mut self, unit: ObjId) {
        let cadence = i64::from(self.arena_cfg.reflex.decide_ms.max(1));
        let offset = i64::from(unit.0) % cadence;
        let mut at = self.now_ms - self.now_ms.rem_euclid(cadence) + offset;
        if at < self.now_ms {
            at = at.saturating_add(cadence);
        }
        self.due
            .schedule(at, DueKind::ReflexTick { unit, tick_seq: 0 });
    }

    /// **One AI unit's decision**, applied at its own arena timestamp.
    ///
    /// Answers whether the unit is still the controller's, which is what
    /// [`crate::arena::step`] reads to decide whether to schedule the next tick.
    pub fn reflex_tick(&mut self, unit: ObjId, t_ms: i64, counters: &mut ReflexCounters) -> bool {
        if self.unit(unit).is_none() {
            return false;
        }
        if self
            .unit(unit)
            .is_none_or(|state| state.controller != Controller::Reflex)
        {
            // Not the controller's. `set_controller` takes a reclaimed seat's ticks
            // off the heap, so this is the race between a release applied in phase 1
            // and a tick already popped in phase 3; it does nothing and does not
            // reschedule, because the adoption schedules its own.
            return false;
        }
        counters.ticks = counters.ticks.saturating_add(1);
        // The memory comes out for the length of the decision so that the deciding
        // half can hold `&self` and `&mut memory` at once; it goes back before a
        // single order is applied, so nothing downstream sees an arena without one.
        let mut memory = std::mem::take(&mut self.reflex);
        let plan = self.decide(unit, t_ms, &mut memory, counters);
        let state = memory
            .unit(unit)
            .map_or(ReflexState::Startup, |row| row.state);
        self.reflex = memory;
        if let Some(slot) = counters.in_state.get_mut(usize::from(state.code())) {
            *slot = slot.saturating_add(1);
        }
        // **16b cut N.2** — the one `order_expired` line, and the squad the next
        // signal will say it is asking about. Journalled before the orders it
        // decided under doctrine are applied, so the black box reads in the order
        // it happened: the link dropped, and then the tank did what it does alone.
        if let Some((squad, objective)) = plan.expired {
            counters.orders_expired = counters.orders_expired.saturating_add(1);
            let hash = self.event_hash();
            let _ = self.ledger.append(
                t_ms,
                EventKind::OrderExpired {
                    unit,
                    squad: SmolStr::new(squad.to_string()),
                    objective: SmolStr::new(objective),
                },
                None,
                hash,
            );
            self.note_order_expired(squad);
        }
        if plan.ordered {
            counters.ordered = counters.ordered.saturating_add(1);
        }
        self.order(unit, &plan, counters);
        true
    }

    /// **Note that a shot of `shooter`'s had no line** (doc 16 §6's `no line`
    /// edge), which is W's `blocked` and `cover` miss reasons read as a fact about
    /// where the shooter is standing.
    ///
    /// Called from `crate::arena::damage`'s re-validation, so the edge is fed by
    /// the black box's own verdict rather than by a second trace of the
    /// controller's.
    pub fn note_no_line(&mut self, shooter: ObjId) {
        if self
            .unit(shooter)
            .is_some_and(|state| state.controller == Controller::Reflex)
            && let Some(row) = self.reflex.units.get_mut(&shooter)
        {
            row.no_line = true;
        }
    }

    /// **Note that `unit` was stopped at `cell`** (16b cut Q.7; doc 16 §3.4 step
    /// 4), which is the `blocked` edge read off the **event** instead of off the
    /// flag.
    ///
    /// [`crate::arena::state::UnitState::blocked`] is cleared at the top of every
    /// substep, so a tick saw a stop only when the blocking substep happened to be
    /// the last one before it — a fact about wall-clock pacing, and therefore
    /// exactly the kind of thing that must not reach hashed state. The `Blocked`
    /// *ledger* event is journalled once per stop, at its own arena timestamp,
    /// with the cell and the cause in hand; this is called from there, and it is
    /// the only way the controller learns of a stop now.
    ///
    /// A stop of any cause throws the route away, as the flag did. A stop with
    /// [`BlockedCause::Occupied`] does two things more: it remembers the cell (see
    /// [`ReflexUnit::avoid`]) and it clears `searched_ms`, so the next tick
    /// searches at once rather than waiting out `repath_ms` against an obstacle
    /// that is not going to move. The re-search is still under
    /// [`ReflexMemory::may_search`]'s per-window budget, so a field that all stops
    /// at once cannot spend more searches than a field that does not.
    pub fn note_blocked(&mut self, unit: ObjId, cell: (u16, u16), cause: BlockedCause, t_ms: i64) {
        if self
            .unit(unit)
            .is_none_or(|state| state.controller != Controller::Reflex)
        {
            return;
        }
        let Some(row) = self.reflex.units.get_mut(&unit) else {
            return;
        };
        row.route.clear();
        if cause != BlockedCause::Occupied {
            return;
        }
        row.searched_ms = None;
        match row.avoid.iter_mut().find(|(at, _)| *at == cell) {
            // A repeat refreshes the stamp rather than taking a second slot: a
            // tank held against one wreck for ten seconds must not push the other
            // seven cells it learned out of the list.
            Some(entry) => entry.1 = t_ms,
            None => {
                if row.avoid.len() >= REFLEX_AVOID_MAX {
                    row.avoid.remove(0);
                }
                row.avoid.push((cell, t_ms));
            }
        }
    }

    /// **Hand a unit to the controller, or take it back** (doc 16 §3.8
    /// "Disconnects"), which is the one thing that moves
    /// [`crate::arena::state::UnitState::controller`] after check-out.
    ///
    /// The tick cadence follows the attribute: a unit handed over joins the stagger
    /// grid from the arena millisecond it was handed over, and a unit handed back
    /// takes its scheduled ticks off the heap with it, so a reclaimed seat costs the
    /// pulse nothing.
    pub fn set_controller(&mut self, unit: ObjId, controller: Controller, state: ReflexState) {
        let was = self.unit(unit).map(|slot| slot.controller);
        if was.is_none() || was == Some(controller) {
            // Already there (or not here at all): adopt into the named state so a
            // second `Released` still points a machine at the entry zone, but do not
            // schedule a second tick for a unit that already has one.
            if controller == Controller::Reflex && was.is_some() {
                self.reflex.adopt(unit, state);
            }
            return;
        }
        if let Some(slot) = self.unit_mut(unit) {
            slot.controller = controller;
        }
        match controller {
            Controller::Reflex => {
                self.reflex.adopt(unit, state);
                self.schedule_reflex_tick(unit);
            }
            Controller::Pilot => {
                self.reflex.release(unit);
                self.due.retain(
                    |kind| !matches!(kind, DueKind::ReflexTick { unit: on, .. } if *on == unit),
                );
            }
        }
    }

    /// **Any unit holds a classified hostile inside its own weapons reach** — doc
    /// 16 §3.1's third `Engaged` trigger, which cut W.1 left `false` because
    /// nobody had bounded a field-wide sweep.
    ///
    /// There is no sweep: it reads the **contacts a platform already holds**, which
    /// the sensor pass minted and bounded, so the cost is one range test per held
    /// classified contact and never a pass over pairs of units. A platform with no
    /// weapon has no reach and answers `false` for every contact it holds, which is
    /// the honest reading of *within weapons range*.
    ///
    /// **16b cut F.3**: the range is to the contact's **plot** — where the
    /// platform last saw the target — for the same reason
    /// [`Arena::contact_within_alert_range`]'s is. How often a pilot is sent a
    /// frame is something a pilot can count, so a tempo read off a position
    /// nobody holds would be a leak of hidden truth.
    #[must_use]
    pub fn hostile_in_weapons_range(&self) -> bool {
        self.units.iter().any(|viewer| {
            let reach = u64::from(viewer.weapon_reach_cm());
            reach != 0
                && self.contacts.of(viewer.id).any(|(_, held)| {
                    held.class.team().is_some_and(|team| team != viewer.team)
                        && self
                            .unit(held.target)
                            .is_some_and(|target| !target.limits.wreck)
                        && held.seen_at.range_from(&viewer.pose) <= reach
                })
        })
    }

    // -----------------------------------------------------------------------
    // Deciding
    // -----------------------------------------------------------------------

    /// Walk the machine one step and say what the unit should be ordered to do.
    #[allow(clippy::too_many_lines)]
    fn decide(
        &self,
        id: ObjId,
        t_ms: i64,
        memory: &mut ReflexMemory,
        counters: &mut ReflexCounters,
    ) -> ReflexPlan {
        let mut plan = ReflexPlan::default();
        let Some(unit) = self.unit(id) else {
            return plan;
        };
        let mut row = memory.unit(id).cloned().unwrap_or_default();
        // **16b cut Q.7**: remembered stops age out, at the top of the tick and
        // in arena time. They are deliberately NOT cleared on a goal change —
        // `Approach` moves its goal every tick, and clearing there would empty the
        // list in exactly the case it exists for.
        row.avoid
            .retain(|(_, at_ms)| t_ms.saturating_sub(*at_ms) < REFLEX_AVOID_TTL_MS);

        // ---- the `Disabled` edge, before anything else ----
        //
        // Doc 16 §3.3: a wreck is no longer the subject of any order, and doc 10
        // 3A's disabled precedes destroyed. Either way the machine stops.
        if unit.limits.wreck || unit.disabled {
            row = ReflexUnit {
                state: ReflexState::Disabled,
                ..ReflexUnit::default()
            };
            memory.units.insert(id, row);
            return plan;
        }

        // ---- the standing order, and the expiry that ends it (16b cut N.2) ----
        //
        // Doc 16 §6: *an order expiring unrenewed drops the squad to doctrine*. The
        // expiry is read **here**, at the machine's own tick and in arena time, and
        // not by a sweep at a pulse boundary: a sweep would drop the order at a
        // different arena millisecond at every tempo, which is the one thing
        // D-16-2 forbids.
        //
        // What doctrine restores: the goal, the route and the target, so a patrol
        // resumes the waypoint graph and an engagement is re-chosen by the utility
        // score. A `Withdraw` the **order** put it in is undone as well, but only
        // when the unit's own health does not ask for one — otherwise the machine
        // would answer an expired order by driving a wreck back into the fight.
        if let Some(standing) = row.order.as_ref()
            && standing.expired_at(t_ms)
        {
            let squad = standing.order.squad;
            let objective = standing.order.objective.as_str();
            row.order = None;
            row.goal = None;
            row.route.clear();
            row.target = None;
            if row.state == ReflexState::Withdraw
                && self.reflex_health_permille(id) >= memory.cfg.withdraw_permille
            {
                row.state = ReflexState::Patrol;
            }
            plan.expired = Some((squad, objective));
        }
        // The order this tick decides under, taken out of the row so the rest of
        // the function can hold it by reference while `row` is borrowed mutably.
        let order: Option<Order> = row.order.as_ref().map(|standing| standing.order.clone());
        plan.ordered = order.is_some();
        // A unit whose squad is not the one the order was written for obeys
        // nothing: a roster that changed under an order is an order with two
        // meanings, and this is the cheap way to have only one.
        let order = order.filter(|order| {
            self.squad_of(id)
                .is_none_or(|roster| roster.id == order.squad)
        });

        // ---- power ----
        match unit.power {
            Power::Off => {
                // **A cold tank starts up, wherever it is going** (16b cut D.3,
                // `B-34`).
                //
                // A tank handed over in `Withdraw` used to stay cold, on the
                // argument that `Withdraw` is a fallback for a tank that is
                // *running*. The playtest showed what that costs: a pilot who
                // `EXIT`s before `STARTUP` leaves a dark tank standing on its
                // entry cell, and because doc 16 §3.8's elimination is *a wreck,
                // or disabled with no weapons*, that tank keeps its side standing
                // until somebody finds it and kills it — a match nobody can end.
                // A machine that brings its own power up and drives home is not
                // inventing an action: it is doing the one thing `Withdraw`
                // means.
                //
                // It stays in `Withdraw` rather than going to `Startup`, because
                // the `Online` arm below re-enters `Patrol` from `Startup` and
                // `Disabled` alone — so a cold `Withdraw` that passed through
                // `Startup` would come up and go looking for a fight it had
                // already left.
                if row.state != ReflexState::Withdraw {
                    row.state = ReflexState::Startup;
                }
                plan.startup = true;
                memory.units.insert(id, row);
                return plan;
            }
            // `STARTUP` has been accepted and `match.startup_ms` has not elapsed.
            // There is nothing to decide and nothing to order.
            //
            // **16b cut D.3 (`B-34`)**: a `Withdraw` keeps its state here for the
            // same reason it keeps it in the arm above. This arm is the whole of
            // the window between the machine's own `Startup` order and the
            // scheduled completion, so writing `Startup` over a `Withdraw` here
            // would hand the `Online` arm below exactly the state it turns into
            // `Patrol` — and a tank that was released cold would come up and go
            // looking for the fight its pilot had just walked out of.
            Power::Starting => {
                if row.state != ReflexState::Withdraw {
                    row.state = ReflexState::Startup;
                }
                memory.units.insert(id, row);
                return plan;
            }
            Power::Online => {
                if row.state == ReflexState::Startup || row.state == ReflexState::Disabled {
                    row.state = ReflexState::Patrol;
                    row.goal = None;
                    row.route.clear();
                }
            }
        }

        // ---- the `lost` edge ----
        //
        // A target the platform no longer holds is not a target: the contact store
        // is the whole of what this unit knows (doc 16 §3.6), so a contact that has
        // expired, or whose unit has left the match, drops the machine back to
        // `Patrol`.
        if let Some((contact, target)) = row.target
            && (self.contacts.resolve(id, contact).map(|held| held.target) != Some(target)
                || self.unit(target).is_none_or(|state| state.limits.wreck))
        {
            row.target = None;
            if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                row.state = ReflexState::Patrol;
                row.goal = None;
                row.route.clear();
            }
        }

        // ---- what the order does to the machine (16b cut N.2) ----
        //
        // An order **steers**: it decides which state the machine is in and what
        // that state is aiming at, and then the state's own body does exactly what
        // it always did. Nothing below this is a second implementation of the
        // machine.
        //
        // - `withdraw` puts it in `Withdraw`, which is the state's own edge reached
        //   by a commander instead of by a health threshold;
        // - `hold` and `advance` are **movement** orders: the squad does not pursue,
        //   so the target is dropped and `Approach`/`Engage` fall back to `Patrol`
        //   with the order's goal. A commander who wants the squad to fight orders
        //   `engage`, which is the whole reason there are four objectives and not
        //   two;
        // - `engage` changes nothing about the states and everything about the
        //   choice inside them: the utility score prefers the hinted contact.
        if let Some(order) = order.as_ref() {
            match order.objective {
                Objective::Withdraw => {
                    if row.state != ReflexState::Withdraw {
                        row.state = ReflexState::Withdraw;
                        row.target = None;
                        row.goal = None;
                        row.route.clear();
                    }
                }
                Objective::Hold | Objective::Advance { .. } => {
                    if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                        row.state = ReflexState::Patrol;
                        row.goal = None;
                        row.route.clear();
                    }
                    row.target = None;
                }
                Objective::Engage { .. } => {}
            }
        }

        // ---- the `Withdraw` edges ----
        //
        // Two of them, and **16b cut Q.7** added the second. The health edge is
        // doc 16 §6's; the *dry* edge is the honest reading of the same sentence
        // for a tank with nothing to shoot — a unit whose magazines are empty, or
        // whose turret has been shot off, is closing on a hostile in order to
        // stand in front of it.
        //
        // The health edge is **not taken by a unit that has already rallied**: a
        // tank gets one return to the fight and then fights to the end, which is
        // what stops a hurt machine oscillating between the entry zone and the
        // line for the rest of the match. A commander's `Objective::Withdraw`
        // (above) overrides either way, rallied or not.
        let health = self.reflex_health_permille(id);
        let dry = is_dry(unit);
        if row.state != ReflexState::Withdraw
            && (dry || (!row.rallied && health < memory.cfg.withdraw_permille))
        {
            row.state = ReflexState::Withdraw;
            row.target = None;
            row.goal = None;
            row.route.clear();
        }

        // ---- the hostile edges ----
        //
        // 16b cut N.2: a movement order suppresses them entirely (see above), so an
        // ordered squad walks past a contact rather than turning on it.
        let may_pursue = !matches!(
            order.as_ref().map(|order| order.objective),
            Some(Objective::Hold | Objective::Advance { .. })
        );
        if may_pursue
            && matches!(
                row.state,
                ReflexState::Patrol | ReflexState::Approach | ReflexState::Engage
            )
        {
            let prefer = self.ordered_target(id, order.as_ref());
            match self.best_hostile(id, memory, health, prefer) {
                Some(found) => {
                    if row.target != Some(found) {
                        row.target = Some(found);
                        row.goal = None;
                        row.route.clear();
                    }
                    if row.state == ReflexState::Patrol {
                        row.state = ReflexState::Approach;
                    }
                }
                None => {
                    row.target = None;
                    if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                        row.state = ReflexState::Patrol;
                        row.goal = None;
                        row.route.clear();
                    }
                }
            }
        }

        // ---- the `blocked` edge (doc 16 §3.4 step 4) ----
        //
        // There is nothing to do here any more. **16b cut Q.7** moved the edge onto
        // the `Blocked` ledger event ([`Arena::note_blocked`]), which sees every
        // stop at its own arena timestamp — the flag this used to read is cleared
        // at the top of every substep, so a tick saw a stop only when the pacing
        // happened to put the two next to each other. One way of learning, not two.
        // ---- the `no line` edge (W's `blocked`/`cover` miss) ----
        //
        // A round of this unit's was stopped by the ground. Standing still and
        // firing again would spend another one the same way, so the goal is thrown
        // away and `Approach` re-plans from here.
        if row.no_line {
            row.no_line = false;
            row.goal = None;
            row.route.clear();
            if row.state == ReflexState::Engage {
                row.state = ReflexState::Approach;
            }
        }

        // ---- the state's own goal and orders ----
        let here = self.definition.terrain.cell_of(&unit.pose);
        match row.state {
            ReflexState::Startup | ReflexState::Disabled => {}
            ReflexState::Patrol => {
                // 16b cut N.2: `advance` replaces the waypoint walk with the cell
                // the commander named; `hold` parks where the unit stands. Both
                // leave the waypoint the machine was walking to alone, so doctrine
                // picks it up again at expiry rather than starting from nothing.
                match order.as_ref().map(|order| order.objective) {
                    Some(Objective::Advance { to }) => {
                        if row.goal != Some(to) {
                            row.goal = Some(to);
                            row.route.clear();
                        }
                    }
                    Some(Objective::Hold) => {
                        row.goal = here;
                        row.route.clear();
                        plan.speed_cms = Some(0);
                    }
                    _ => self.patrol_goal(id, &mut row, here),
                }
            }
            // **16b cut F.3**: every range, goal and heading below is read off the
            // unit's **own contact** — the plot, where its sensors or its mesh
            // last saw the target — and never off the target's live pose. A
            // reflex unit drives to where it last saw something, which is what a
            // crew does; before this cut it closed on a tank it could not see,
            // through a wall, with perfect knowledge, for the whole time to live.
            //
            // `reflex_has_line` is deliberately **not** on the plot: it is the
            // same trace the round will take, so asking it about the live target
            // is asking "would this shot arrive", which is physics and not
            // knowledge.
            ReflexState::Approach => {
                if let Some((contact, target)) = row.target {
                    let reach = u64::from(unit.weapon_reach_cm());
                    let plot = self.plot_of(id, contact);
                    let range = plot.map_or(u64::MAX, |plot| plot.range_from(&unit.pose));
                    let hold = band_hold_cm(unit, memory.cfg.engage_band_permille, order.as_ref());
                    if reach != 0
                        && range <= order_reach_cm(unit, order.as_ref()).max(1)
                        && self.reflex_has_line(id, target, counters)
                    {
                        row.state = ReflexState::Engage;
                        row.goal = None;
                        row.route.clear();
                        self.engage_orders(id, contact, order.as_ref(), &mut plan);
                    } else {
                        row.goal = plot
                            .and_then(|plot| self.stand_off_cell(id, &plot, hold))
                            .or(here);
                    }
                }
            }
            ReflexState::Engage => {
                if let Some((contact, _)) = row.target {
                    let reach = u64::from(unit.weapon_reach_cm());
                    let plot = self.plot_of(id, contact);
                    let range = plot.map_or(u64::MAX, |plot| plot.range_from(&unit.pose));
                    if reach == 0 || range > order_reach_cm(unit, order.as_ref()) {
                        row.state = ReflexState::Approach;
                        let hold =
                            band_hold_cm(unit, memory.cfg.engage_band_permille, order.as_ref());
                        row.goal = plot
                            .and_then(|plot| self.stand_off_cell(id, &plot, hold))
                            .or(here);
                    } else {
                        // Holding the band: stop, turn onto the plot, shoot.
                        row.goal = here;
                        row.route.clear();
                        plan.speed_cms = Some(0);
                        plan.heading_mdeg = plot.map(|plot| plot.bearing_from(&unit.pose));
                        self.engage_orders(id, contact, order.as_ref(), &mut plan);
                    }
                }
            }
            // **16b cut Q.7**: `Withdraw` is no longer terminal. It has two new
            // edges inside it and one out of it, and none of them is a state flip
            // on a live line — see [`ReflexUnit::stand_until_ms`] for why.
            ReflexState::Withdraw => {
                if row.goal.is_none() {
                    row.goal = self.entry_cell(id).or(here);
                }
                // (a) **cornered**. A hostile it holds, classified, inside the
                // reach an order allows it, with a line: it stops where it is,
                // turns onto the plot and shoots — **without leaving `Withdraw`
                // and without touching the goal or the route**, so the run home
                // resumes from where it stopped rather than from a fresh search.
                // A tank with nothing to fire, or one told to hold its fire, is
                // never cornered: see [`Arena::withdraw_stand`].
                let cornered = self.withdraw_stand(id, memory, health, order.as_ref(), counters);
                if let Some((contact, plot)) = cornered {
                    row.stand_until_ms = Some(t_ms.saturating_add(
                        REFLEX_STAND_TICKS.saturating_mul(i64::from(memory.cfg.decide_ms)),
                    ));
                    row.quiet_since_ms = None;
                    plan.speed_cms = Some(0);
                    plan.heading_mdeg = Some(plot.bearing_from(&unit.pose));
                    self.engage_orders(id, contact, order.as_ref(), &mut plan);
                } else if row.stand_until_ms.is_some_and(|until| t_ms < until) {
                    // The dwell: the line has gone for the moment, and a tank
                    // that turned its back on the last three seconds of it would
                    // be turning round again on the next tick.
                    row.quiet_since_ms = None;
                    plan.speed_cms = Some(0);
                } else {
                    row.stand_until_ms = None;
                    // (b) **rally**. Home (or with nowhere to go) and with nothing
                    // hostile on the board for `rally_ms` of arena time: back to
                    // `Patrol`, once. A unit with nothing to fire never rallies —
                    // it has no fight to come back to.
                    let arrived = match (here, row.goal) {
                        (Some(here), Some(goal)) => {
                            here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) <= ARRIVE_CELLS
                        }
                        (_, None) => true,
                        _ => false,
                    };
                    if self.holds_a_hostile(id) || !arrived || dry {
                        row.quiet_since_ms = None;
                    } else {
                        let since = *row.quiet_since_ms.get_or_insert(t_ms);
                        if t_ms.saturating_sub(since) >= i64::from(memory.cfg.rally_ms) {
                            row.state = ReflexState::Patrol;
                            row.rallied = true;
                            row.goal = None;
                            row.route.clear();
                            row.quiet_since_ms = None;
                        }
                    }
                }
            }
        }

        // ---- the route, and the steer ----
        if !matches!(row.state, ReflexState::Startup | ReflexState::Disabled)
            && plan.speed_cms.is_none()
        {
            self.follow(id, t_ms, &mut row, here, memory, counters, &mut plan);
        }
        memory.units.insert(id, row);
        plan
    }

    /// `Patrol`'s goal: the node it is walking to, advanced when it arrives.
    fn patrol_goal(&self, id: ObjId, row: &mut ReflexUnit, here: Option<(u16, u16)>) {
        let graph = &self.definition.waypoints;
        if graph.is_empty() {
            // No graph authored: a patrol with nowhere to go holds where it is,
            // which is the honest answer and not a random walk.
            row.goal = None;
            row.route.clear();
            return;
        }
        let Some((x, y)) = here else {
            row.goal = None;
            return;
        };
        let mut node = match row.waypoint {
            Some(node) if graph.node(node).is_some() => node,
            _ => {
                let Some(node) = graph.nearest(x, y) else {
                    return;
                };
                row.from_waypoint = None;
                node
            }
        };
        let arrived = graph
            .node(node)
            .is_some_and(|at| at.x.abs_diff(x).max(at.y.abs_diff(y)) <= ARRIVE_CELLS);
        if arrived {
            let neighbours = graph.neighbours(node);
            let onward: Vec<u16> = neighbours
                .iter()
                .copied()
                .filter(|next| Some(*next) != row.from_waypoint)
                .collect();
            let choices: &[u16] = if onward.is_empty() {
                neighbours
            } else {
                &onward
            };
            if let Some(next) = choices
                .get(usize::try_from(id.0).unwrap_or(0) % choices.len().max(1))
                .copied()
            {
                row.from_waypoint = Some(node);
                node = next;
                row.goal = None;
                row.route.clear();
            }
        }
        row.waypoint = Some(node);
        let goal = graph.node(node).map(|at| (at.x, at.y));
        if row.goal != goal {
            row.goal = goal;
            row.route.clear();
        }
    }

    /// `Engage`'s two discrete actions: the lock, and the trigger.
    ///
    /// Reconciliation (b) of the cut brief: the readiness, the rounds, the power
    /// and the lock are **read** here rather than staged and refused, so the
    /// resolver is handed an order it can accept. The weapon chosen is the first
    /// in component order that is ready, loaded and in reach — component order,
    /// because that is the order `WEAPONSTATUS` prints and the order a pilot would
    /// read down.
    fn engage_orders(
        &self,
        id: ObjId,
        contact: ContactId,
        order: Option<&Order>,
        plan: &mut ReflexPlan,
    ) {
        let Some(unit) = self.unit(id) else {
            return;
        };
        if unit.lock != Some(contact) {
            plan.lock = Some(contact);
        }
        if unit.power != Power::Online {
            return;
        }
        // 16b cut N.2: `hold_fire` is a constraint on the **trigger** and on
        // nothing else — the machine still closes, turns and locks, so the squad is
        // one order away from shooting rather than one decision cycle plus a lock.
        if order.is_some_and(|order| order.constraints.hold_fire) {
            return;
        }
        // 16b cut F.3: the plot's range, which is the range the gunner has.
        let range = self
            .plot_of(id, contact)
            .map_or(u64::MAX, |plot| plot.range_from(&unit.pose));
        let ceiling = order_reach_cm(unit, order);
        if range > ceiling {
            return;
        }
        // **16b cut Q.7**: and the heat. The controller reads the platform's
        // ceiling the way `crate::arena::weapons`'s acceptance does — the `heat`
        // cell against its own maximum — and skips a weapon whose shot would take
        // it there, so it does not feed an `Overheated` refusal into the resolver
        // every tick and then sit with a hot gun. A unit already at the ceiling
        // fires nothing at all, which is the same sentence read at zero cost. A
        // unit whose content declares no heat is never held back here, which is
        // every stock tank.
        plan.fire = unit
            .weapons
            .iter()
            .find(|weapon| {
                unit.weapon_ready(&weapon.id, self.now_ms)
                    && unit.weapon_ammo(&weapon.id) > 0
                    && u64::from(weapon.max_range_cm) >= range
                    && !would_overheat(unit, &weapon.id)
            })
            .map(|weapon| weapon.id.clone());
    }

    /// **Whether a withdrawing unit is cornered** (16b cut Q.7), and by which
    /// contact: the hostile it would fight if it had to, with the plot to turn
    /// onto.
    ///
    /// The candidate is the utility score's own best hostile — the same chooser
    /// `Approach` uses, so a `Withdraw` that turns to fight turns onto the same
    /// tank it would have closed on — and the two tests after it are `Approach`'s
    /// own, in `Approach`'s own order: the **plot's** range against what the order
    /// allows (16b cut F.3: the range the gunner has), and then one trace. The
    /// trace is paid only when the range test has already passed, so a withdrawing
    /// tank with nothing near it costs a `Withdraw` tick exactly what it cost
    /// before this cut.
    ///
    /// **A unit that cannot shoot is never cornered.** The stand stops a tank in
    /// the open and turns it onto the plot; a tank that would then stage no shot
    /// has stopped in front of a gun for nothing, which is the parked target this
    /// cut exists to remove and not a stand. The two reasons that hold for the
    /// whole of a stand — the magazines are empty ([`is_dry`]), and a commander
    /// wrote `hold_fire` — are asked here, before the candidate. A weapon merely
    /// on its cycle is *not* one of them: that one comes back within the second,
    /// and a tank that gave up the stand for it would be turning round twice a
    /// second.
    fn withdraw_stand(
        &self,
        id: ObjId,
        memory: &ReflexMemory,
        health_permille: u32,
        order: Option<&Order>,
        counters: &mut ReflexCounters,
    ) -> Option<(ContactId, SeenPose)> {
        let unit = self.unit(id)?;
        if is_dry(unit) || order.is_some_and(|order| order.constraints.hold_fire) {
            return None;
        }
        let (contact, target) = self.best_hostile(id, memory, health_permille, None)?;
        let plot = self.plot_of(id, contact)?;
        if plot.range_from(&unit.pose) > order_reach_cm(unit, order).max(1) {
            return None;
        }
        if !self.reflex_has_line(id, target, counters) {
            return None;
        }
        Some((contact, plot))
    }

    /// **Whether this platform holds any live hostile at all** (16b cut Q.7) —
    /// the question `Withdraw`'s rally asks, and the widest reading of it: any
    /// classified contact of another side whose unit is not a wreck, at any range
    /// and with or without a line.
    ///
    /// It is the contact store and nothing else (doc 16 §3.6), so a unit that
    /// cannot see the enemy rallies and a unit being watched from a ridge does
    /// not.
    fn holds_a_hostile(&self, id: ObjId) -> bool {
        self.unit(id).is_some_and(|unit| {
            self.contacts.of(id).any(|(_, held)| {
                matches!(held.class, Classification::Classified { team } if team != unit.team)
                    && self
                        .unit(held.target)
                        .is_some_and(|target| !target.limits.wreck)
            })
        })
    }

    /// **The object an order points this unit at**, or `None` (16b cut N.2).
    ///
    /// The hint is written in the **commander's** contact ids, because that is the
    /// only contact store the request was built from; it is resolved there into an
    /// object, and the caller then looks for *this* unit's own contact on that
    /// object. A unit that holds nothing on it prefers nothing and takes its
    /// doctrine's best target, which is doc 16 §3.6's observer rule read through an
    /// order: a commander may point, and a tank that cannot see still cannot see.
    fn ordered_target(&self, id: ObjId, order: Option<&Order>) -> Option<ObjId> {
        let order = order?;
        let hint = order.preferred_contact()?;
        let commander = self.squad_of(id)?.commander;
        self.contacts
            .resolve(commander, hint)
            .map(|held| held.target)
    }

    /// Whether a shot at `target` would have a line at all — the test `Approach`
    /// makes before it declares itself `Engage`.
    ///
    /// One trace, counted, and only when the range already says a shot is possible:
    /// the controller does not probe the map, it checks the one line it is about to
    /// spend a round on.
    fn reflex_has_line(&self, id: ObjId, target: ObjId, counters: &mut ReflexCounters) -> bool {
        counters.traces = counters.traces.saturating_add(1);
        let (sight, cover) = self.fire_line(id, target);
        sight.is_clear_or_partial() && cover != Cover::Full
    }

    /// **Doc 16 §6's utility score**: the classified hostile this unit should close
    /// on, or `None` when it holds none.
    ///
    /// Lower is better. Three weighted per-mille terms, all integers:
    ///
    /// * **range** — the range as a fraction of this unit's own reach, so a
    ///   platform with a long gun is not drawn off by something far away that it
    ///   could still hit;
    /// * **facet** — the band the candidate presents toward us
    ///   ([`facet_band_seen`], off the contact's own plot),
    ///   which is the difference between a tank showing its rear and one showing
    ///   its front;
    /// * **health** — this unit's own, which is the same addend for every
    ///   candidate and therefore orders nothing. It is in the score because doc 16
    ///   §6 puts it there, and it is the number the `Withdraw` edge reads.
    ///
    /// **Ties break by unit id**, ascending, because `Arena::units` is kept in that
    /// order and nothing here draws.
    fn best_hostile(
        &self,
        id: ObjId,
        memory: &ReflexMemory,
        health_permille: u32,
        prefer: Option<ObjId>,
    ) -> Option<(ContactId, ObjId)> {
        let unit = self.unit(id)?;
        // **A platform with nothing to shoot with has no target.** `Approach` and
        // `Engage` are both about closing to a weapon's band, and a tank that
        // carries none — an unarmed unit, or one whose turret has been shot off —
        // would be closing on a hostile in order to sit in front of it. It patrols
        // instead, which is doc 16 §6's machine with the states that mean nothing
        // to it left out.
        let reach = u64::from(unit.weapon_reach_cm());
        if reach == 0 {
            return None;
        }
        let health_term = memory
            .cfg
            .weight_health
            .saturating_mul(health_permille.min(1_000));
        let mut best: Option<(u64, ObjId, ContactId)> = None;
        for (contact, held) in self.contacts.of(id) {
            let Classification::Classified { team } = held.class else {
                continue;
            };
            if team == unit.team {
                continue;
            }
            let Some(target) = self.unit(held.target) else {
                continue;
            };
            if target.limits.wreck {
                continue;
            }
            // **16b cut F.3**: scored off the plot — the range to where this
            // platform last saw the candidate, and the face it was showing then.
            // A utility function that read the live pose would rank a target
            // nobody can see by where it actually is.
            let range = held.seen_at.range_from(&unit.pose);
            let range_permille = u32::try_from(range.saturating_mul(1_000) / reach)
                .unwrap_or(u32::MAX)
                .min(1_000);
            let exposure = match facet_band_seen(&held.seen_at, unit) {
                "rear" => EXPOSURE_REAR,
                "front" => EXPOSURE_FRONT,
                _ => EXPOSURE_FLANK,
            };
            // **16b cut N.2**: the order's hint wins outright, and only outright.
            // It is a zero score rather than a weight, because a commander who
            // names a target has said something the utility function cannot: what
            // the *squad* is for. A hint the unit cannot see never reaches here
            // (`ordered_target` resolved it to nothing), so this can only ever
            // choose among contacts the unit already holds.
            let score = if prefer == Some(target.id) {
                0
            } else {
                u64::from(memory.cfg.weight_range.saturating_mul(range_permille))
                    .saturating_add(u64::from(memory.cfg.weight_facet.saturating_mul(exposure)))
                    .saturating_add(u64::from(health_term))
            };
            let candidate = (score, target.id, *contact);
            if best.is_none_or(|(had, unit, _)| (score, target.id) < (had, unit)) {
                best = Some(candidate);
            }
        }
        best.map(|(_, target, contact)| (contact, target))
    }

    /// **Where this unit's own contact says its target is** (16b cut F.3), or
    /// `None` when it no longer holds it.
    ///
    /// The one place the controller turns a contact into a position, so every
    /// state below reads the plot and none of them can reach past it to the unit.
    fn plot_of(&self, id: ObjId, contact: ContactId) -> Option<SeenPose> {
        self.contacts.resolve(id, contact).map(|held| held.seen_at)
    }

    /// The cell to stand in to hold `hold_cm` of range on `at` — the goal
    /// `Approach` drives to.
    ///
    /// The point on the line from the plot toward *here*, at the holding range.
    /// A cell off the window or impassable falls back to the plot's own cell,
    /// and the path search's `unreachable` edge is what catches that.
    ///
    /// **16b cut F.3** made `at` the contact's plot rather than the target's
    /// object: an approach is toward the last known position, and a unit that
    /// arrives there and finds nothing is a unit that has lost its contact.
    fn stand_off_cell(&self, id: ObjId, at: &SeenPose, hold_cm: u64) -> Option<(u16, u16)> {
        let from = self.unit(id)?;
        let range = at.range_from(&from.pose);
        let want = i64::try_from(hold_cm.max(1)).unwrap_or(i64::MAX);
        if range == 0 {
            return self.definition.terrain.cell_of(&from.pose);
        }
        let range = i64::try_from(range).unwrap_or(i64::MAX).max(1);
        let dx = from.pose.x_cm.saturating_sub(at.x_cm);
        let dy = from.pose.y_cm.saturating_sub(at.y_cm);
        let x = at.x_cm.saturating_add(dx.saturating_mul(want) / range);
        let y = at.y_cm.saturating_add(dy.saturating_mul(want) / range);
        let cell = i64::from(self.definition.terrain.cell_cm().max(1));
        let (Ok(cx), Ok(cy)) = (
            u16::try_from(x.div_euclid(cell)),
            u16::try_from(y.div_euclid(cell)),
        ) else {
            return at.cell(&self.definition.terrain);
        };
        if self
            .definition
            .terrain
            .get(cx, cy)
            .is_some_and(crate::arena::state::TerrainCell::passable)
        {
            Some((cx, cy))
        } else {
            at.cell(&self.definition.terrain)
        }
    }

    /// `Withdraw`'s goal: the nearest passable cell of this unit's **own** entry
    /// zone (doc 16 §6: *toward the entry zone*).
    fn entry_cell(&self, id: ObjId) -> Option<(u16, u16)> {
        let unit = self.unit(id)?;
        let (here_x, here_y) = self.definition.terrain.cell_of(&unit.pose)?;
        let mut best: Option<(u32, (u16, u16))> = None;
        let mut read = 0_usize;
        for zone in self
            .definition
            .entry_zones
            .iter()
            .filter(|zone| zone.team == unit.team)
        {
            let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
            let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
            // The clamped corner first: it is the answer almost always, and it
            // costs one probe.
            let clamped = (here_x.clamp(x0, x1), here_y.clamp(y0, y1));
            if self
                .definition
                .terrain
                .get(clamped.0, clamped.1)
                .is_some_and(crate::arena::state::TerrainCell::passable)
            {
                return Some(clamped);
            }
            for y in y0..=y1 {
                for x in x0..=x1 {
                    read = read.saturating_add(1);
                    if read > MAX_ENTRY_SCAN {
                        return best.map(|(_, cell)| cell).or(Some(clamped));
                    }
                    if !self
                        .definition
                        .terrain
                        .get(x, y)
                        .is_some_and(crate::arena::state::TerrainCell::passable)
                    {
                        continue;
                    }
                    let distance = u32::from(x.abs_diff(here_x)) + u32::from(y.abs_diff(here_y));
                    if best.is_none_or(|(far, _)| distance < far) {
                        best = Some((distance, (x, y)));
                    }
                }
            }
        }
        best.map(|(_, cell)| cell)
    }

    /// **Armour plus internals, per mille of what the unit was authored with**
    /// (reconciliation (c) of the cut brief), read off the M32 cells on
    /// `UnitState::unit`.
    ///
    /// Every `armor` and `internals` cell on the unit, current against maximum,
    /// each measured from its own floor: a facet whose armour is at `min` counts
    /// zero and a section at full counts its whole span. A unit with no such cell
    /// at all — cut A's fixtures — reads `1000`, a unit in perfect health, which is
    /// the only answer that does not make an unmodelled hull withdraw at once.
    #[must_use]
    pub fn reflex_health_permille(&self, id: ObjId) -> u32 {
        let Some(unit) = self.unit(id) else {
            return 0;
        };
        if unit.limits.wreck {
            return 0;
        }
        let mut have: i64 = 0;
        let mut full: i64 = 0;
        for (addr, cell) in unit.unit.cells() {
            let name = match addr {
                mush_tactics::CellAddr::Facet { cell, .. }
                | mush_tactics::CellAddr::Section { cell, .. } => cell.as_str(),
                mush_tactics::CellAddr::Unit { .. } | mush_tactics::CellAddr::Component { .. } => {
                    continue;
                }
            };
            if name != crate::definitions::unit::ARMOR_CELL
                && name != crate::definitions::unit::INTERNALS_CELL
            {
                continue;
            }
            have = have.saturating_add(cell.current.saturating_sub(cell.min).max(0));
            full = full.saturating_add(cell.max.saturating_sub(cell.min).max(0));
        }
        if full <= 0 {
            return 1_000;
        }
        u32::try_from(have.saturating_mul(1_000) / full)
            .unwrap_or(1_000)
            .min(1_000)
    }

    /// Keep the route fresh and turn it into a heading and a speed.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &self,
        id: ObjId,
        t_ms: i64,
        row: &mut ReflexUnit,
        here: Option<(u16, u16)>,
        memory: &mut ReflexMemory,
        counters: &mut ReflexCounters,
        plan: &mut ReflexPlan,
    ) {
        let Some(unit) = self.unit(id) else {
            return;
        };
        let (Some(here), Some(goal)) = (here, row.goal) else {
            plan.speed_cms = Some(0);
            return;
        };
        // Cells already behind the unit leave the route; the one it is standing in
        // is one of them.
        while row.route.first().is_some_and(|cell| *cell == here) {
            row.route.remove(0);
        }
        if here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) <= ARRIVE_CELLS {
            row.route.clear();
            plan.speed_cms = Some(0);
            return;
        }
        // A route whose next cell is not a neighbour of this one is a route the
        // unit has wandered off (it was stopped, or it was pushed round an
        // obstacle); it is thrown away rather than followed sideways.
        if row
            .route
            .first()
            .is_some_and(|cell| cell.0.abs_diff(here.0).max(cell.1.abs_diff(here.1)) > 1)
        {
            row.route.clear();
            // **16b cut Q.7**: and the search that route came from is spent. A
            // unit that has been pushed off its route is standing somewhere the
            // route was never computed from, and making it wait out `repath_ms`
            // before it may say so is what left a tank crawling sideways past an
            // obstacle for four seconds at a time.
            row.searched_ms = None;
        }
        if row.route.is_empty() {
            let fresh_enough = row
                .searched_ms
                .is_some_and(|at| t_ms.saturating_sub(at) < i64::from(memory.cfg.repath_ms));
            if fresh_enough {
                counters.path_deferred = counters.path_deferred.saturating_add(1);
            } else if memory.may_search(t_ms) {
                row.searched_ms = Some(t_ms);
                counters.path_queries = counters.path_queries.saturating_add(1);
                let budget = memory.cfg.path_max_cells;
                let scale = memory.min_move_cost;
                // **16b cut Q.7**: the cells this unit has been stopped at, at
                // most `REFLEX_AVOID_MAX` of them, built once per search.
                let avoid: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
                // **16b cut X.6**: the unit's own slope limit goes into the
                // search, so the route the controller follows is a route the
                // crossing will accept. A route over an edge motion then refuses
                // is a tank driving at a bluff and re-planning at it every
                // `repath_ms` for the rest of the match.
                match path_with(
                    &self.definition.terrain,
                    &mut memory.scratch,
                    here,
                    goal,
                    budget,
                    scale,
                    unit.slope_limit_pct,
                    &avoid,
                ) {
                    Ok(found) => {
                        counters.path_cells = counters
                            .path_cells
                            .saturating_add(u64::from(found.expanded));
                        row.route = found.cells;
                    }
                    Err(why) => {
                        // **The `unreachable` edge** (doc 16 §6): over budget, no
                        // route, or ground that is not ground. The goal is given up
                        // and the nearest waypoint takes its place; a patrol whose
                        // own node is what failed steps onward along the graph
                        // instead, so the fallback cannot be the same refusal twice.
                        counters.path_failures = counters.path_failures.saturating_add(1);
                        let _ = why;
                        self.fall_back_to_waypoint(id, row, here);
                    }
                }
            } else {
                counters.path_deferred = counters.path_deferred.saturating_add(1);
            }
        }
        // Steer at whatever the route (or, with none, the goal) says next.
        let cell_cm = i64::from(self.definition.terrain.cell_cm().max(1));
        let routed = row.route.first().copied();
        let aim = routed.unwrap_or(goal);
        let to_x = i64::from(aim.0).saturating_mul(cell_cm) + cell_cm / 2;
        let to_y = i64::from(aim.1).saturating_mul(cell_cm) + cell_cm / 2;
        let heading = bearing_of(
            to_x.saturating_sub(unit.pose.x_cm),
            to_y.saturating_sub(unit.pose.y_cm),
        );
        // **16b cut Q.7** (closes 16b cut X.6's deviation 13): with no route, the
        // goal is a *direction* and nobody has checked the ground between here and
        // it. The one cell the unit would actually cross first is checked with the
        // crossing's own rule, and a unit that would be refused it waits for its
        // search rather than driving at a cliff or back into the cell it was just
        // stopped at. With a route the question is already answered — the search
        // asked it of every edge it planned.
        if routed.is_none() && !self.first_step_is_open(unit, here, heading, &row.avoid) {
            plan.speed_cms = Some(0);
            return;
        }
        plan.heading_mdeg = Some(heading);
        plan.speed_cms = Some(i32::try_from(unit.mobility.max_speed_cms).unwrap_or(i32::MAX));
    }

    /// **Whether the one cell on `heading` is a cell this unit may enter** (16b
    /// cut Q.7), asked exactly as [`crate::arena::motion`]'s step 4 will ask it.
    ///
    /// The neighbour is the octant of the bearing — the eight compass directions,
    /// each owning 45 degrees — because that is the cell the crossing will test
    /// first. Then four questions, in the crossing's own order: it is on the
    /// window, its class is passable, the edge onto it is within this unit's slope
    /// limit **either way**, and on a diagonal its shoulders are not a cliff
    /// ([`shoulder_refuses`], the search's own rule read out of the search's own
    /// module). And one of this cut's: it is not a cell the unit has been stopped
    /// at.
    ///
    /// A unit with no slope limit at all is refused by no slope, exactly as
    /// `path` and `motion` refuse it by none.
    fn first_step_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        heading_mdeg: u32,
        avoid: &[((u16, u16), i64)],
    ) -> bool {
        let terrain = &self.definition.terrain;
        let (dx, dy) = octant_step(heading_mdeg);
        let (Some(nx), Some(ny)) = (
            u16::try_from(i32::from(here.0).saturating_add(dx)).ok(),
            u16::try_from(i32::from(here.1).saturating_add(dy)).ok(),
        ) else {
            return false;
        };
        if (nx, ny) == here {
            return true;
        }
        let (Some(here_cell), Some(to_cell)) = (terrain.get(here.0, here.1), terrain.get(nx, ny))
        else {
            return false;
        };
        if !to_cell.passable() {
            return false;
        }
        if avoid.iter().any(|(cell, _)| *cell == (nx, ny)) {
            return false;
        }
        edge_is_open(terrain, unit, here_cell, to_cell, here, (nx, ny))
    }

    /// The `unreachable` edge's landing: the nearest waypoint, or the next one
    /// along the graph when the nearest is what could not be reached.
    fn fall_back_to_waypoint(&self, id: ObjId, row: &mut ReflexUnit, here: (u16, u16)) {
        row.route.clear();
        let graph = &self.definition.waypoints;
        let Some(node) = graph.nearest(here.0, here.1) else {
            // No graph to fall back on: give the goal up and hold. A unit that
            // cannot get there and has nowhere else to be does not drive at a wall.
            row.goal = None;
            return;
        };
        let at = graph.node(node).map(|node| (node.x, node.y));
        let node = if at == row.goal {
            let neighbours = graph.neighbours(node);
            neighbours
                .get(usize::try_from(id.0).unwrap_or(0) % neighbours.len().max(1))
                .copied()
                .unwrap_or(node)
        } else {
            node
        };
        row.from_waypoint = row.waypoint;
        row.waypoint = Some(node);
        row.goal = graph.node(node).map(|node| (node.x, node.y));
    }

    // -----------------------------------------------------------------------
    // Ordering
    // -----------------------------------------------------------------------

    /// Put the plan through the pilot's own door.
    ///
    /// Each order is an [`Intent`] on the channel that carries it, at the unit's
    /// **own current epoch** — the controller does not hold authority of its own,
    /// it uses whatever the world last published for the seat, which is `0` for a
    /// tank nobody was ever in and the released seat's new epoch for one handed
    /// over mid-match. `apply_intent` is what accepts or refuses it; a discrete
    /// action it queued is taken straight back out and resolved here, at this
    /// tick's arena timestamp (see the module head for why not at phase 4).
    fn order(&mut self, id: ObjId, plan: &ReflexPlan, counters: &mut ReflexCounters) {
        let mut seq = 0_u64;
        let mut orders: Vec<IntentKind> = Vec::new();
        if plan.startup {
            orders.push(IntentKind::Startup);
        }
        if let Some(mdeg) = plan.heading_mdeg {
            orders.push(IntentKind::Heading { mdeg });
        }
        if let Some(cms) = plan.speed_cms {
            orders.push(IntentKind::Speed { cms });
        }
        if let Some(contact) = plan.lock {
            orders.push(IntentKind::Lock { contact });
        }
        if let Some(weapon) = plan.fire.clone() {
            orders.push(IntentKind::Fire { weapon });
        }
        for kind in orders {
            let channel = kind.channel_required();
            seq = seq.saturating_add(1);
            let intent = Intent {
                unit: id,
                channel,
                epoch: self.epochs.current(id, channel),
                kind,
                seq,
            };
            counters.intents = counters.intents.saturating_add(1);
            if apply_intent(self, &intent) != IntentOutcome::Queued {
                continue;
            }
            // `apply_intent` pushed exactly this order to the back of the unit's
            // queue. It comes straight back out: leaving it there would hand it to
            // phase 4, at the pulse's end, which is the arena millisecond this
            // module exists to avoid.
            if let Some(queue) = self.pending.get_mut(&id) {
                queue.pop_back();
                if queue.is_empty() {
                    self.pending.remove(&id);
                }
            }
            match &intent.kind {
                IntentKind::Startup => {
                    let _ = resolve_startup(self, id);
                }
                IntentKind::Lock { contact } => {
                    let _ = self.resolve_lock(id, *contact);
                }
                IntentKind::Fire { weapon } => {
                    if self.resolve_fire(id, weapon).is_ok() {
                        counters.shots = counters.shots.saturating_add(1);
                    }
                }
                IntentKind::Heading { .. } | IntentKind::Speed { .. } | IntentKind::Order(_) => {}
            }
        }
    }
}

/// **The furthest range an order lets a unit shoot at**, centimetres (16b cut
/// N.2).
///
/// `None`, or an order with no band, is the platform's own reach and nothing
/// changes. A band caps it at that band's ceiling, so `close` is a squad told to
/// let the enemy come to it and `far` caps nothing, which is a commander saying
/// "no restriction" without a fourth word.
fn order_reach_cm(unit: &UnitState, order: Option<&Order>) -> u64 {
    let reach = u64::from(unit.weapon_reach_cm());
    match order.and_then(|order| order.constraints.max_range_band) {
        // 16b cut F.3: the band's ceiling is read against the authored bands of
        // the weapon a bare `FIRE` fires, which is the same list the pilot's own
        // band word is read against. The list is only built on this arm, so an
        // unconstrained squad allocates nothing.
        Some(band) => reach.min(band.ceiling_cm(&unit.fire_bands_cm())),
        None => reach,
    }
}

/// **The 8-neighbour step nearest a bearing** (16b cut Q.7).
///
/// Bearing zero is north and [`bearing_of`] measures clockwise, so the eight
/// octants are 45 degrees wide and centred on the compass points: the octant is
/// `(mdeg + 22,500) / 45,000` modulo eight, and the table below is the step each
/// one names in cell coordinates (`+y` north, `+x` east).
fn octant_step(heading_mdeg: u32) -> (i32, i32) {
    const STEPS: [(i32, i32); 8] = [
        (0, 1),
        (1, 1),
        (1, 0),
        (1, -1),
        (0, -1),
        (-1, -1),
        (-1, 0),
        (-1, 1),
    ];
    let octant = ((heading_mdeg.saturating_add(22_500) / 45_000) % 8) as usize;
    STEPS.get(octant).copied().unwrap_or((0, 1))
}

/// **Whether one edge is crossable for this unit** (16b cut Q.7), which is
/// [`crate::arena::motion`]'s step 4 and [`crate::arena::path::path_with`]'s edge
/// rule asked of a single step.
fn edge_is_open(
    terrain: &ArenaTerrain,
    unit: &UnitState,
    here_cell: &crate::arena::state::TerrainCell,
    to_cell: &crate::arena::state::TerrainCell,
    here: (u16, u16),
    to: (u16, u16),
) -> bool {
    let Some(limit) = unit.slope_limit_pct else {
        return true;
    };
    let cell_cm = u32::from(terrain.cell_cm());
    let Some(kind) = slope::step_kind(here, to) else {
        return true;
    };
    if slope::too_steep(
        Some(limit),
        slope::steepness_pct(here_cell, to_cell, kind, cell_cm),
    ) {
        return false;
    }
    if kind == mush_atlas::physics::StepKind::Diagonal
        && shoulder_refuses(terrain, Some(limit), here_cell, to_cell, here, to)
    {
        return false;
    }
    true
}

/// **Whether a platform that *had* a gun has nothing left to fire** (16b cut
/// Q.7), which is the `Withdraw` edge doc 16 §6's health threshold does not
/// cover.
///
/// Two halves, and the first is what keeps it honest. A tank that came into the
/// match with no weapon at all is not *dry*: it is a spotter, a target or a
/// harness mover, and [`Arena::best_hostile`] already says what the machine does
/// with one — *it patrols, which is doc 16 §6's machine with the states that mean
/// nothing to it left out*. The second half is `remove_weapons` and an empty
/// magazine read as the one fact they are: there is no shot this platform can
/// take, so closing on a hostile would be closing in order to stand in front of
/// it.
fn is_dry(unit: &UnitState) -> bool {
    unit.armed_at_checkout()
        && !unit
            .weapons
            .iter()
            .any(|weapon| unit.weapon_ammo(&weapon.id) > 0)
}

/// **Whether firing `weapon` would take this platform past its heat ceiling**
/// (16b cut Q.7; doc 16 §9 item 1).
///
/// The same two numbers `crate::arena::weapons`'s acceptance reads — the `heat`
/// cell and its own maximum — plus the weapon's `heat_per_shot`. A platform with
/// no heat at all answers `false` for every weapon.
///
/// **It is the engine's own line and not one shot short of it.**
/// [`crate::arena::state::UnitState::overheated`] refuses at the ceiling *and not
/// merely past it*: a shot that would carry a unit past the maximum leaves it
/// exactly at it, and the **next** shot is the one refused. So the controller
/// declines a shot that would land the unit **above** the ceiling and takes the
/// one that lands it on it — otherwise a weapon whose `heat_per_shot` equals its
/// platform's whole ceiling would be a gun a pilot could fire from cold and a
/// machine could never fire at all, which is the two halves of one platform
/// disagreeing about whether the gun works. A unit already at the ceiling is
/// asked the engine's own question and answers `true` for every weapon **that
/// makes heat** — **16b cut D.2** (`B-16`) took the zero out of that set, in both
/// halves at once: the ceiling is what a hot shot would add to, and a weapon
/// whose `heat_per_shot` is nought adds nothing, so a platform that has cooked
/// itself may still fire its cold guns. `crate::arena::weapons`'s acceptance
/// carries the same clause.
///
/// A platform whose content declares heat but carries no `heat` **cell** reads
/// `None` here and is held back by nothing, because that is the platform
/// [`crate::arena::state::UnitState::overheated`] answers `false` for however hot
/// its weapons are: the engine would take every one of those shots.
fn would_overheat(unit: &UnitState, weapon: &str) -> bool {
    let Some(heat) = unit.heat.as_ref() else {
        return false;
    };
    let Some(now) = unit.heat_now() else {
        return false;
    };
    // 16b cut D.2 (`B-16`): a cold gun is never held back, at the ceiling or
    // anywhere else.
    let per_shot = heat.per_shot(weapon);
    if per_shot == 0 {
        return false;
    }
    if now >= heat.max {
        return true;
    }
    now.saturating_add(per_shot) > heat.max
}

/// **The range `Approach` holds**, centimetres (16b cut N.2): doctrine's own
/// `engage_band_permille` of the reach, capped by the order's band.
fn band_hold_cm(unit: &UnitState, engage_band_permille: u32, order: Option<&Order>) -> u64 {
    let doctrine =
        u64::from(unit.weapon_reach_cm()).saturating_mul(u64::from(engage_band_permille)) / 1_000;
    doctrine.min(order_reach_cm(unit, order))
}

/// The per-unit rows, for [`crate::arena::hash::state_hash`] — the one reader
/// outside this module that needs the machines in order and nothing else.
impl ReflexMemory {
    /// Every machine's state, keyed by unit, in unit-id order.
    pub(crate) fn rows(&self) -> impl Iterator<Item = (&ObjId, &ReflexUnit)> {
        self.units.iter()
    }
}

/// Everything a unit-level test of the controller needs to reach.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_of, pose, tank};

    /// **16b cut D.2** (`B-16`): the controller's copy of the engine's heat
    /// question answers `false` for a weapon that makes no heat, at the ceiling
    /// and below it — otherwise a cooked platform's cold gun is one a pilot may
    /// fire and a machine may not, which is the two halves of one platform
    /// disagreeing about whether the gun works.
    #[test]
    fn d2_a_weapon_that_makes_no_heat_never_holds_the_controller_back() {
        let mut unit = tank(ObjId(4), Team::Red, pose(1_500, 1_500));
        unit.unit = hot_tactical_unit(ObjId(4));
        let mut arena = arena_of(vec![unit]);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.heat = Some(crate::arena::state::HeatState {
                section: SmolStr::new("hull"),
                max: 60,
                per_s: 3,
                per_shot: [(SmolStr::new("laser"), 20)].into_iter().collect(),
                carry_milli: 0,
            });
        }
        let asked = |arena: &Arena| -> (bool, bool) {
            let Some(state) = arena.unit(ObjId(4)) else {
                panic!("the fixture tank");
            };
            (
                would_overheat(state, "laser"),
                would_overheat(state, "machine_gun"),
            )
        };
        // Cold: neither is held back.
        assert_eq!(asked(&arena), (false, false));
        // One pulse short of the ceiling: the hot one's own shot would carry it
        // past, the cold one adds nothing.
        set_heat(&mut arena, ObjId(4), 50);
        assert_eq!(asked(&arena), (true, false));
        // **At** the ceiling, which is the branch this cut changed: the engine
        // refuses the hot gun and takes the cold one, and so does the machine.
        set_heat(&mut arena, ObjId(4), 60);
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

    /// The fixture's one-section unit **with a heat cell on its hull** — the
    /// shipped laser tank's shape, small enough to build here, because
    /// `test_support`'s own unit declares none and a controller that cannot read
    /// a heat cell is the documented case this test is not about.
    fn hot_tactical_unit(id: ObjId) -> mush_tactics::TacticalUnit {
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

    /// Put the heat cell at `value` through one plan, which is the only way a
    /// unit's cells move (D-16-6).
    fn set_heat(arena: &mut Arena, unit: ObjId, value: i64) {
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

    #[test]
    fn the_states_round_trip_their_words_and_codes() {
        for (at, state) in ReflexState::ALL.iter().enumerate() {
            assert_eq!(usize::from(state.code()), at);
            assert!(!state.as_str().is_empty());
        }
        assert_eq!(Controller::Pilot.code(), 0);
        assert_eq!(Controller::Reflex.code(), 1);
        assert_eq!(Controller::Reflex.as_str(), "reflex");
    }

    #[test]
    fn adopting_a_unit_twice_does_not_restart_its_machine() {
        let cfg = ReflexConfig::default();
        let mut memory = ReflexMemory::new(&cfg, 100);
        memory.adopt(ObjId(4), ReflexState::Startup);
        if let Some(row) = memory.units.get_mut(&ObjId(4)) {
            row.state = ReflexState::Patrol;
            row.goal = Some((3, 3));
        }
        memory.adopt(ObjId(4), ReflexState::Patrol);
        assert_eq!(
            memory.unit(ObjId(4)).and_then(|row| row.goal),
            Some((3, 3)),
            "a second adopt into the state it is already in keeps the goal"
        );
        memory.adopt(ObjId(4), ReflexState::Withdraw);
        assert_eq!(
            memory.unit(ObjId(4)).map(|row| row.state),
            Some(ReflexState::Withdraw)
        );
        assert_eq!(memory.unit(ObjId(4)).and_then(|row| row.goal), None);
        memory.release(ObjId(4));
        assert!(memory.unit(ObjId(4)).is_none());
    }

    #[test]
    fn the_path_budget_is_a_window_of_arena_time_and_not_a_pulse() {
        let cfg = ReflexConfig {
            decide_ms: 1_000,
            max_path_queries: 2,
            ..ReflexConfig::default()
        };
        let mut memory = ReflexMemory::new(&cfg, 100);
        assert!(memory.may_search(0));
        assert!(memory.may_search(400));
        assert!(!memory.may_search(900), "the window's budget is spent");
        // The next window opens at 1,000 ms of arena time, whatever the tempo.
        assert!(memory.may_search(1_000));
        assert!(memory.may_search(1_999));
        assert!(!memory.may_search(1_999));
    }

    /// **16b cut Q.7**: the remembered-stop list is bounded, refreshes rather
    /// than duplicating, and only an `Occupied` stop puts anything in it.
    #[test]
    fn the_avoided_cells_are_bounded_and_a_repeat_refreshes_its_stamp() {
        let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.controller = Controller::Reflex;
        }
        arena.reflex.adopt(ObjId(4), ReflexState::Patrol);
        // A search this tick already paid for, so the clearing below is a fact
        // about `note_blocked` and not about how `adopt` leaves a fresh row.
        if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
            row.searched_ms = Some(0);
        }
        // Nine distinct cells into a list of eight: the oldest goes.
        for n in 0..9_u16 {
            arena.note_blocked(ObjId(4), (n, 1), BlockedCause::Occupied, i64::from(n) * 100);
        }
        let row = arena.reflex.unit(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        assert_eq!(row.avoid.first().map(|(cell, _)| *cell), Some((1, 1)));
        assert_eq!(row.avoid.last().map(|(cell, _)| *cell), Some((8, 1)));
        assert_eq!(row.searched_ms, None, "the next tick searches");

        // A repeat moves the stamp and takes no second slot.
        arena.note_blocked(ObjId(4), (1, 1), BlockedCause::Occupied, 9_000);
        let row = arena.reflex.unit(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        assert_eq!(row.avoid.first().copied(), Some(((1, 1), 9_000)));

        // Every other cause throws the route away and remembers nothing: the
        // ground is not going to move out of the way, and the search already
        // knows about it.
        if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
            row.route = vec![(2, 2)];
            row.avoid.clear();
        }
        for cause in [
            BlockedCause::Impassable,
            BlockedCause::Edge,
            BlockedCause::TooSteep,
        ] {
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                row.searched_ms = Some(10_000);
            }
            arena.note_blocked(ObjId(4), (3, 3), cause, 10_000);
            let row = arena.reflex.unit(ObjId(4)).cloned().unwrap_or_default();
            assert!(row.route.is_empty(), "{cause:?} throws the route away");
            assert!(row.avoid.is_empty(), "{cause:?} remembers nothing");
            assert_eq!(
                row.searched_ms,
                Some(10_000),
                "{cause:?} is ground, not traffic: the search it just paid for stands"
            );
        }

        // And a unit that is not the controller's learns nothing at all.
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.controller = Controller::Pilot;
        }
        arena.note_blocked(ObjId(4), (7, 7), BlockedCause::Occupied, 11_000);
        assert!(
            arena
                .reflex
                .unit(ObjId(4))
                .is_some_and(|row| row.avoid.is_empty())
        );
    }

    /// **16b cut Q.7**: each of the four new row fields is authoritative state,
    /// so each of them moves [`crate::arena::hash::state_hash`].
    #[test]
    fn each_new_row_field_moves_the_state_hash() {
        let build = || {
            let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
            arena.reflex.adopt(ObjId(4), ReflexState::Withdraw);
            arena
        };
        let base = crate::arena::hash::state_hash(&build());
        let mut moved: Vec<u64> = Vec::new();
        for edit in [
            0_u8, // the avoided cell
            1,    // its stamp
            2,    // stand_until_ms
            3,    // quiet_since_ms
            4,    // rallied
        ] {
            let mut arena = build();
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                match edit {
                    0 => row.avoid.push(((3, 4), 1_000)),
                    1 => row.avoid.push(((3, 4), 2_000)),
                    2 => row.stand_until_ms = Some(1_500),
                    3 => row.quiet_since_ms = Some(1_500),
                    _ => row.rallied = true,
                }
            }
            let hash = crate::arena::hash::state_hash(&arena);
            assert_ne!(hash, base, "edit {edit} did not move the hash");
            assert!(
                !moved.contains(&hash),
                "edit {edit} folded the same bytes as an earlier one"
            );
            moved.push(hash);
        }
    }

    #[test]
    fn the_octant_of_a_bearing_is_the_cell_the_crossing_will_test() {
        for (mdeg, step) in [
            (0_u32, (0_i32, 1_i32)),
            (22_499, (0, 1)),
            (22_500, (1, 1)),
            (45_000, (1, 1)),
            (90_000, (1, 0)),
            (135_000, (1, -1)),
            (180_000, (0, -1)),
            (225_000, (-1, -1)),
            (270_000, (-1, 0)),
            (315_000, (-1, 1)),
            (359_999, (0, 1)),
        ] {
            assert_eq!(octant_step(mdeg), step, "{mdeg} mdeg");
        }
    }

    #[test]
    fn an_unmodelled_hull_reads_as_whole_and_a_wreck_reads_as_gone() {
        let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        // Cut A's fixture carries one armour cell and one internals cell, both full.
        assert_eq!(arena.reflex_health_permille(ObjId(4)), 1_000);
        assert_eq!(arena.reflex_health_permille(ObjId(9)), 0, "no such unit");
        if let Some(state) = arena.unit_mut(ObjId(4)) {
            state.limits.wreck = true;
        }
        assert_eq!(arena.reflex_health_permille(ObjId(4)), 0);
    }
}
