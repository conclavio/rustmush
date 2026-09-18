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
use crate::arena::path::{PathScratch, path};
use crate::arena::perception::bearing_of;
use crate::arena::resolution::facet_band;
use crate::arena::state::{Arena, Classification, ContactId, Controller, DueKind, Power};
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
    #[must_use]
    pub fn hostile_in_weapons_range(&self) -> bool {
        self.units.iter().any(|viewer| {
            let reach = u64::from(viewer.weapon_reach_cm());
            reach != 0
                && self.contacts.of(viewer.id).any(|(_, held)| {
                    held.class.team().is_some_and(|team| team != viewer.team)
                        && self.unit(held.target).is_some_and(|target| {
                            !target.limits.wreck
                                && crate::arena::index::range_cm(&viewer.pose, &target.pose)
                                    <= reach
                        })
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
                // **A tank handed over cold in `Withdraw` stays cold.** Doc 16
                // §3.8's released seat is a fallback for a tank that is *running* —
                // a pilot who exits before `STARTUP` leaves a cold tank, and an AI
                // that brought it up to drive it a hundred metres home would be
                // inventing an action nobody asked for. `Withdraw` is where the
                // release puts it and `Withdraw` is where it stays.
                if row.state == ReflexState::Withdraw {
                    memory.units.insert(id, row);
                    return plan;
                }
                row.state = ReflexState::Startup;
                plan.startup = true;
                memory.units.insert(id, row);
                return plan;
            }
            // `STARTUP` has been accepted and `match.startup_ms` has not elapsed.
            // There is nothing to decide and nothing to order.
            Power::Starting => {
                row.state = ReflexState::Startup;
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

        // ---- the `Withdraw` edge ----
        let health = self.reflex_health_permille(id);
        if row.state != ReflexState::Withdraw && health < memory.cfg.withdraw_permille {
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

        // ---- the `blocked` edge (doc 16 §3.4 step 4, A.2's flag) ----
        //
        // The unit is held against an obstacle, an occupied cell or the window's
        // edge. Whatever route it was following ends at a wall, so it is thrown
        // away and the next search starts from where the unit actually is.
        if unit.blocked {
            row.route.clear();
        }
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
            ReflexState::Approach => {
                if let Some((contact, target)) = row.target {
                    let reach = u64::from(unit.weapon_reach_cm());
                    let range = self
                        .unit(target)
                        .map(|to| crate::arena::index::range_cm(&unit.pose, &to.pose))
                        .unwrap_or(u64::MAX);
                    let hold = band_hold_cm(
                        unit.weapon_reach_cm(),
                        memory.cfg.engage_band_permille,
                        order.as_ref(),
                    );
                    if reach != 0
                        && range <= order_reach_cm(unit.weapon_reach_cm(), order.as_ref()).max(1)
                        && self.reflex_has_line(id, target, counters)
                    {
                        row.state = ReflexState::Engage;
                        row.goal = None;
                        row.route.clear();
                        self.engage_orders(id, contact, target, order.as_ref(), &mut plan);
                    } else {
                        row.goal = self.stand_off_cell(id, target, hold).or(here);
                    }
                }
            }
            ReflexState::Engage => {
                if let Some((contact, target)) = row.target {
                    let reach = u64::from(unit.weapon_reach_cm());
                    let range = self
                        .unit(target)
                        .map(|to| crate::arena::index::range_cm(&unit.pose, &to.pose))
                        .unwrap_or(u64::MAX);
                    if reach == 0 || range > order_reach_cm(unit.weapon_reach_cm(), order.as_ref())
                    {
                        row.state = ReflexState::Approach;
                        let hold = band_hold_cm(
                            unit.weapon_reach_cm(),
                            memory.cfg.engage_band_permille,
                            order.as_ref(),
                        );
                        row.goal = self.stand_off_cell(id, target, hold).or(here);
                    } else {
                        // Holding the band: stop, turn onto the target, shoot.
                        row.goal = here;
                        row.route.clear();
                        plan.speed_cms = Some(0);
                        plan.heading_mdeg = self.unit(target).map(|to| {
                            bearing_of(to.pose.x_cm - unit.pose.x_cm, to.pose.y_cm - unit.pose.y_cm)
                        });
                        self.engage_orders(id, contact, target, order.as_ref(), &mut plan);
                    }
                }
            }
            ReflexState::Withdraw => {
                if row.goal.is_none() {
                    row.goal = self.entry_cell(id).or(here);
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
        target: ObjId,
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
        let range = self
            .unit(target)
            .map(|to| crate::arena::index::range_cm(&unit.pose, &to.pose))
            .unwrap_or(u64::MAX);
        let ceiling = order_reach_cm(unit.weapon_reach_cm(), order);
        if range > ceiling {
            return;
        }
        plan.fire = unit
            .weapons
            .iter()
            .find(|weapon| {
                unit.weapon_ready(&weapon.id, self.now_ms)
                    && unit.weapon_ammo(&weapon.id) > 0
                    && u64::from(weapon.max_range_cm) >= range
            })
            .map(|weapon| weapon.id.clone());
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
    /// * **facet** — the band the candidate presents toward us ([`facet_band`]),
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
            let range = crate::arena::index::range_cm(&unit.pose, &target.pose);
            let range_permille = u32::try_from(range.saturating_mul(1_000) / reach)
                .unwrap_or(u32::MAX)
                .min(1_000);
            let exposure = match facet_band(target, unit) {
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

    /// The cell to stand in to hold `hold_cm` of range on `target` — the goal
    /// `Approach` drives to.
    ///
    /// The point on the line from the target toward *here*, at the holding range.
    /// A cell off the window or impassable falls back to the target's own cell,
    /// and the path search's `unreachable` edge is what catches that.
    fn stand_off_cell(&self, id: ObjId, target: ObjId, hold_cm: u64) -> Option<(u16, u16)> {
        let from = self.unit(id)?;
        let to = self.unit(target)?;
        let range = crate::arena::index::range_cm(&from.pose, &to.pose);
        let want = i64::try_from(hold_cm.max(1)).unwrap_or(i64::MAX);
        if range == 0 {
            return self.definition.terrain.cell_of(&from.pose);
        }
        let range = i64::try_from(range).unwrap_or(i64::MAX).max(1);
        let dx = from.pose.x_cm.saturating_sub(to.pose.x_cm);
        let dy = from.pose.y_cm.saturating_sub(to.pose.y_cm);
        let x = to.pose.x_cm.saturating_add(dx.saturating_mul(want) / range);
        let y = to.pose.y_cm.saturating_add(dy.saturating_mul(want) / range);
        let cell = i64::from(self.definition.terrain.cell_cm().max(1));
        let (Ok(cx), Ok(cy)) = (
            u16::try_from(x.div_euclid(cell)),
            u16::try_from(y.div_euclid(cell)),
        ) else {
            return self.definition.terrain.cell_of(&to.pose);
        };
        if self
            .definition
            .terrain
            .get(cx, cy)
            .is_some_and(crate::arena::state::TerrainCell::passable)
        {
            Some((cx, cy))
        } else {
            self.definition.terrain.cell_of(&to.pose)
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
                match path(
                    &self.definition.terrain,
                    &mut memory.scratch,
                    here,
                    goal,
                    budget,
                    scale,
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
        let aim = row.route.first().copied().unwrap_or(goal);
        let to_x = i64::from(aim.0).saturating_mul(cell_cm) + cell_cm / 2;
        let to_y = i64::from(aim.1).saturating_mul(cell_cm) + cell_cm / 2;
        plan.heading_mdeg = Some(bearing_of(
            to_x.saturating_sub(unit.pose.x_cm),
            to_y.saturating_sub(unit.pose.y_cm),
        ));
        plan.speed_cms = Some(i32::try_from(unit.mobility.max_speed_cms).unwrap_or(i32::MAX));
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
fn order_reach_cm(reach_cm: u32, order: Option<&Order>) -> u64 {
    let reach = u64::from(reach_cm);
    match order.and_then(|order| order.constraints.max_range_band) {
        Some(band) => reach.min(band.ceiling_cm(reach_cm)),
        None => reach,
    }
}

/// **The range `Approach` holds**, centimetres (16b cut N.2): doctrine's own
/// `engage_band_permille` of the reach, capped by the order's band.
fn band_hold_cm(reach_cm: u32, engage_band_permille: u32, order: Option<&Order>) -> u64 {
    let doctrine = u64::from(reach_cm).saturating_mul(u64::from(engage_band_permille)) / 1_000;
    doctrine.min(order_reach_cm(reach_cm, order))
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
