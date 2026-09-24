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
//! **`Patrol` is where a machine that has nothing to fight decides what to do**,
//! and since 16b cut E.4 it has four answers, taken in this order: a movement
//! order's cell; **the search**, the cell of the plot of the target it has just
//! lost, for `reflex.search_ms`; **the hunt**, a cell of the *other* side's entry
//! zone, once `reflex.hunt_after_ms` has passed with nothing hostile held at all;
//! and then the arena pack's waypoint graph, which is what it walked before. A
//! movement order drops both clocks' goals and restarts the hunt's wait. The
//! search and the hunt close 16c `B-04`'s *endgame that never ends*: the
//! waypoint walk is a fixed cycle per unit id, so two sides' loops need never
//! meet, and a withdrawn machine parks in an entry zone no loop passes through.
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
//! divergence read once more. (**16b cut M.2**, 16c `T-02`, made that sentence
//! true: until then the resolvers read `now_ms`, which the substep had already
//! advanced to its end when the tick ran; they now take the tick's `t_ms`, and
//! the planner reads readiness against the same clock.)
//!
//! The one thing phase 4 keeps is the hash discipline: a substep that runs reflex
//! work takes **one** [`crate::arena::hash::state_hash`] and every action inside
//! it carries that one hash, the same way phase 4 takes one for the whole phase
//! (`Arena::action_hash`). Since 16b cut M.3 (16c `T-23`) both take it lazily, at
//! the first line a tick or the phase journals, so a substep whose ticks order
//! nothing takes none.
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

use crate::arena::index::range_between;
use crate::arena::intents::{
    Intent, IntentKind, IntentOutcome, IntentSource, apply_intent, resolve_startup,
};
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
use crate::deliberation::order::{Objective, Order, RangeBand, StandingOrder};

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

/// **The most cells one unit remembers being stopped at** (16b cut Q.7), the
/// **least recently stopped at** dropped (16b cut E.3, `B-43`).
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
///
/// **A cell a machine is still held against does not age** (16b cut M.3):
/// `Arena::refresh_held_avoid` re-stamps it at every tick while
/// [`crate::arena::state::UnitState::held`] names it, so the thirty seconds run
/// from the last tick the unit was still pushing at it.
pub const REFLEX_AVOID_TTL_MS: i64 = 30_000;

/// **Remember `cell` on a machine's avoid list at `t_ms`** (16b cut Q.7, its
/// eviction 16b cut E.3's, shared since 16b cut M.3 by the stop's own edge and
/// the tick's held refresh): a listed cell has its stamp refreshed in place, and
/// a new one takes a slot, evicting the least recently stopped at when the list
/// is full.
fn remember_avoid(row: &mut ReflexUnit, cell: (u16, u16), t_ms: i64) {
    match row.avoid.iter_mut().find(|(at, _)| *at == cell) {
        // A listed cell refreshes its stamp rather than taking a second slot: a
        // tank held against one wreck for ten seconds must not push the other
        // seven cells it learned out of the list.
        Some(entry) => entry.1 = t_ms,
        None => {
            // **16b cut E.3 (16c `B-43`)**: the list evicts the **least recently
            // stopped at**, not the one that happens to sit at index zero.
            // Insertion order and recency are not the same order — a refresh
            // moves an entry's stamp in place — so the old rule dropped exactly
            // the cell the unit kept being stopped at, which is the one cell it
            // must not forget. Ties go to the lowest index, so the choice is a
            // function of the list and not of its scan.
            if row.avoid.len() >= REFLEX_AVOID_MAX {
                let oldest = row
                    .avoid
                    .iter()
                    .enumerate()
                    .min_by_key(|(index, (_, at_ms))| (*at_ms, *index))
                    .map(|(index, _)| index);
                if let Some(index) = oldest {
                    row.avoid.remove(index);
                }
            }
            row.avoid.push((cell, t_ms));
        }
    }
}

/// **How far short of `engage_cm` an `Approach` aims its stand-off cell**, in
/// cells (16b cut E.1).
///
/// Three, and the number is the arrive window plus the cell quantisation.
/// [`ARRIVE_CELLS`] is one cell Chebyshev — 14 m on a diagonal at the shipped
/// 10 m cell — and the goal a search is given is a cell, whose centre is up to
/// half a cell off the point `Arena::stand_off_cell` projected. So a goal
/// placed *at* `engage_cm` leaves a unit that has arrived a few metres outside
/// the band: stopped, in `Approach`, never firing. Three cells is enough slack
/// that an arrived unit is inside `engage_cm` and little enough that the machine
/// is still holding a band rather than closing onto the target.
pub const STAND_OFF_MARGIN_CELLS: u16 = 3;

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
    /// **Looking for something to fight.** Its goal is the first of these that
    /// answers (16b cut E.4):
    ///
    /// 1. **a movement order** — `Advance`'s cell, or `Hold`'s "where you
    ///    stand" (16b cut N.2);
    /// 2. **the search** — [`ReflexUnit::last_plot`], while it is younger than
    ///    [`crate::config::ReflexConfig::search_ms`] and the machine has not
    ///    arrived at it: where it last saw the target it has just lost;
    /// 3. **the hunt** — [`ReflexUnit::hunt`], the other side's entry zone,
    ///    once [`crate::config::ReflexConfig::hunt_after_ms`] has passed with
    ///    nothing hostile held at all;
    /// 4. **the waypoint walk** — `Arena::patrol_goal`, the arena pack's
    ///    graph, which is what `Patrol` was and still is when the three above
    ///    have nothing to say.
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
    /// [`REFLEX_AVOID_MAX`] entries, the **least recently stopped at** dropped
    /// (16b cut E.3, `B-43`: evicting by insertion order dropped the cell the
    /// unit kept being stopped at, because a refresh moves a stamp in place);
    /// a new stop at a listed cell refreshes its stamp, and so does every tick
    /// while the unit is still held against that cell (16b cut M.3: the `blocked`
    /// line fires once per edge now, so re-collisions no longer refresh it); an
    /// entry older than [`REFLEX_AVOID_TTL_MS`] is dropped at the top of a tick.
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
    ///
    /// **16b cut E.2 (16c `B-40`)**: it is also held at `None` for the whole of
    /// a live `Objective::Withdraw`. A unit a commander sent home does not rally
    /// out of the order — the order would put it back in `Withdraw` on the next
    /// tick — so the clock does not even start until the order has expired, and
    /// the `rally_ms` a hurt unit then waits is counted from that tick and not
    /// from some quiet minute in the middle of the order.
    pub quiet_since_ms: Option<i64>,
    /// **This unit has already come back into the fight once** (16b cut Q.7).
    ///
    /// The health edge is not taken twice: a tank that withdrew, waited out its
    /// quiet and rallied fights to the end. A commander's `Objective::Withdraw`
    /// still overrides it, and so does having nothing left to fire.
    ///
    /// **16b cut E.2 (16c `B-40`)**: a standing `Objective::Withdraw` is
    /// therefore never what sets it. Under such an order the rally does not run
    /// at all, so the one return a machine is owed is still owed when the order
    /// ends — which is the right way round: the commander spent the order, not
    /// the tank's one comeback.
    pub rallied: bool,
    /// **The squad order it is under** (16b cut N.2; doc 16 §6 "Deliberation"),
    /// with the arena milliseconds it was accepted at and runs out at.
    ///
    /// `None` is doctrine, which is what doc 16 §6 calls *the fallback when nothing
    /// arrives*: not idle, not a hold — the machine below, exactly as it ran before
    /// an order arrived and exactly as it runs after one expires.
    pub order: Option<StandingOrder>,
    /// **Where this machine last saw its target, and when** (16b cut E.4; 16c
    /// `B-04`): the cell of the target's plot, and the arena millisecond of the
    /// tick that read it.
    ///
    /// Written by the tick that ends with a target and a plot on the window, in
    /// `Approach`, in `Engage` and in a `Withdraw` that stood and fought. Read by
    /// `Patrol`, which drives to it for
    /// [`crate::config::ReflexConfig::search_ms`] — so the `lost` edge is a crew
    /// going to the last place it had something, and not a crew turning its back
    /// on it.
    ///
    /// It is the machine's **own** last plot and never the target's live cell
    /// (doc 16 §1, and 16b cut F.3's rule applied once more): a target that moves
    /// after the contact expires does not drag the search after it.
    ///
    /// Cleared by arrival within [`ARRIVE_CELLS`], by an age past `search_ms`
    /// (an age of exactly `search_ms` still searches), by `search_ms == 0`, by
    /// entering `Withdraw`, by every tick of a movement order (`Advance`,
    /// `Hold`), and by the `unreachable` edge when it was the search's own goal
    /// that no route reached.
    pub last_plot: Option<((u16, u16), i64)>,
    /// **Since when this machine has held nothing hostile at all** (16b cut E.4;
    /// 16c `B-04`), arena milliseconds, or `None` while it holds one.
    ///
    /// The clock [`crate::config::ReflexConfig::hunt_after_ms`] runs against. It
    /// is `Patrol`'s: every other state restarts it at the tick it is read, and
    /// so does every tick of a movement order, so a machine that has just come
    /// out of a fight, or out from under an order, starts the wait over.
    pub unheld_since_ms: Option<i64>,
    /// **The cell of the other side's entry zone this machine is crossing to**
    /// (16b cut E.4; 16c `B-04`), or `None`.
    ///
    /// Set once the clock above has run out, from `Arena::far_entry_cell`,
    /// which reads the arena definition's zones and no unit's pose. Cleared, and
    /// the clock restarted, by arrival within [`ARRIVE_CELLS`] (which also
    /// forgets the waypoint the walk was on, so it resumes from the node nearest
    /// the far edge), by the `unreachable` edge on it, by every tick of a
    /// movement order, and by leaving `Patrol` for any reason.
    pub hunt: Option<(u16, u16)>,
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
    /// **Edges refused by the slope rule** (16b cut U.1; 16c `T-35`): every edge
    /// a search refused as too steep either way or by the shoulder rule
    /// ([`crate::arena::path::PathScratch::slope_refusals`]), plus every time the
    /// follower's one-cell test (`Arena::first_step_is_open`) found the cell in
    /// front open to the class and the avoid list and refused it by the same
    /// edge rule. What the harness asserts non-zero to prove the cohort's
    /// locomotion met the relief; a counter, not state, so it moves no hash.
    pub slope_refusals: u32,
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
        self.slope_refusals = self.slope_refusals.saturating_add(other.slope_refusals);
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
    /// Which `decide_ms` window of arena time the budget below belongs to.
    window: i64,
    /// Searches already spent in that window.
    spent: u32,
}

impl ReflexMemory {
    /// An empty memory with this match's doctrine frozen in, exactly as the two
    /// configuration blocks are frozen into [`Arena`].
    ///
    /// **16b cut M.4** (16c `T-15`): the window's cheapest cell, which 16b cut
    /// N.1 kept here for the A\* heuristic, moved to
    /// [`Arena::min_move_cost`](crate::arena::state::Arena::min_move_cost), one
    /// copy for the search and the speed ceiling both.
    #[must_use]
    pub fn new(cfg: &ReflexConfig) -> ReflexMemory {
        ReflexMemory {
            units: BTreeMap::new(),
            scratch: PathScratch::default(),
            cfg: cfg.clone(),
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
            // **16b cut E.4**: and a clean memory of what it last saw and how
            // long it has seen nothing. A seat released into `Withdraw` must not
            // inherit its pilot's last contact as a place to drive to, and the
            // hunt's clock starts when the machine takes the tank over.
            row.last_plot = None;
            row.unheld_since_ms = None;
            row.hunt = None;
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
// The three distances a tick fights at
// ---------------------------------------------------------------------------

/// **What one machine's ranges are this tick** (16b cut E.1; 16c `B-05`,
/// `B-39`, `T-32`, `T-46`, `T-47`), all integer centimetres, computed once from
/// the unit, the doctrine and the order.
///
/// Before this cut every range test the controller made was against
/// [`UnitState::weapon_reach_cm`] — the *longest* reach the platform carries,
/// 3 km on a stock tank — on a map where the sight is 600 m and a mesh hop adds
/// 300 m. All of them were therefore always true: `Approach` became `Engage` at
/// the first line anywhere, `Engage` could never fall back, and a hurt machine
/// stood and fought at any range. The one place the doctrine band was *used*
/// was worse: 700 per mille of 3 km is 2.1 km, and `stand_off_cell` projects
/// that back from the plot, so the "stand-off" goal was up to 1.2 km **behind**
/// the machine.
///
/// So every distance here is priced off [`UnitState::fight_inner_cm`] and
/// [`UnitState::fight_limit_cm`] — the lesser of the platform's own sight and
/// the band of the weapon it would fire.
///
/// # The doctrine band is applied after the order's cap, and not before it
///
/// `engage_cm` is `engage_band_permille` of the reach the **order** allows, and
/// `release_cm` is that reach itself. Capping the two separately collapses them
/// to one distance under two of the commander's three words (`close` gives
/// 200 / 200 on a stock tank, `effective` 400 / 400), and a target drifting
/// about that one distance flips the machine between stopping to shoot and
/// driving to a stand-off cell every tick. Applied after the cap the hysteresis
/// is the same three tenths under every order, and the machine may fire across
/// the whole of its `Engage` region — so it never stands in `Engage` at a range
/// it may not shoot from.
///
/// The stock tank, engage / release / stand-off goal:
///
/// | order band | `engage_cm` | `release_cm` | stand-off `want` |
/// |---|---|---|---|
/// | none, or `far` | 420 m | 600 m | 390 m |
/// | `effective` | 280 m | 400 m | 250 m |
/// | `close` | 140 m | 200 m | 110 m |
///
/// `stand_cm` is 200 m under all three. **16b cut AA.1**: a machine that cannot
/// close ([`UnitState::cannot_close`]) enters `Engage` at `release_cm` and not at
/// `engage_cm`, so its region is the whole row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ranges {
    /// Where `Approach` stops and turns: the doctrine band of the allowed reach —
    /// for a machine that can close (16b cut AA.1).
    engage_cm: u64,
    /// Where `Engage` gives up and closes again — **and the ceiling
    /// [`Arena::engage_orders`] will fire out to**, so the two cannot disagree.
    release_cm: u64,
    /// What a `Withdraw` still on its way home will turn and fight inside: the
    /// `close` band, capped by the order **and by `release_cm`**.
    ///
    /// The last cap is not redundant. The `close` band is a third of the *last*
    /// band edge held to the sight, and `release_cm` is the *first* one held to
    /// the same sight, so a gun whose inner band is under a third of its outer
    /// reach — nothing the game ships, and an ordinary shape for content to
    /// author — would otherwise let a hurt machine stop in front of a hostile
    /// that [`Arena::engage_orders`]' own ceiling then refuses to fire at. That
    /// is the parked target 16b cut Q.7 was paid to remove, arrived at by
    /// doctrine instead of by a pilot, and [`Arena::withdraw_stand`]'s doc says
    /// a unit that cannot shoot is never cornered.
    stand_cm: u64,
    /// The furthest this platform fights at at all; `0` is a platform with
    /// nothing loaded, which has no target.
    limit_cm: u64,
}

/// [`Ranges`] for one unit under one order.
fn ranges(unit: &UnitState, cell_cm: u16, cfg: &ReflexConfig, order: Option<&Order>) -> Ranges {
    // The loaded weapon's own edges, built once: `ceiling_cm` reads the last of
    // them and the sight, exactly as the pilot's band word does. A platform with
    // nothing loaded has an empty list, and every distance below falls to nought.
    let bands = unit
        .loaded_weapon()
        .map_or_else(Vec::new, crate::arena::state::WeaponSummary::bands_cm);
    let sight = unit.sight_cm(cell_cm);
    let order_cap = order
        .and_then(|order| order.constraints.max_range_band)
        .map_or(u64::MAX, |band| band.ceiling_cm(&bands, sight));
    let cap_inner = unit.fight_inner_cm(cell_cm).min(order_cap);
    Ranges {
        engage_cm: cap_inner.saturating_mul(u64::from(cfg.engage_band_permille)) / 1_000,
        release_cm: cap_inner,
        stand_cm: RangeBand::Close
            .ceiling_cm(&bands, sight)
            .min(order_cap)
            .min(cap_inner),
        limit_cm: unit.fight_limit_cm(cell_cm),
    }
}

/// **How much range an `Approach` aims to keep on the plot**, centimetres (16b
/// cut E.1; 16c `B-05`) — the argument [`Arena::stand_off_cell`] projects back
/// from the plot toward the machine.
///
/// Floor first, `min` with the range **last**, and the order is the whole of the
/// function: the literal `engage_cm − margin` underflows a `u64` for a short band
/// or a small sight, and a floor applied *after* the `min` can exceed `range`,
/// which projects the goal behind the machine — `B-05` written a second time. As
/// written the answer is never above `range`, and never nought unless `range` is.
fn stand_off_want(engage_cm: u64, cell_cm: u64, range_cm: u64) -> u64 {
    engage_cm
        .saturating_sub(u64::from(STAND_OFF_MARGIN_CELLS).saturating_mul(cell_cm))
        .max(cell_cm)
        .min(range_cm)
}

// ---------------------------------------------------------------------------
// The tick
// ---------------------------------------------------------------------------

impl Arena {
    /// **The first [`DueKind::ReflexTick`] of every unit the check-out handed to
    /// the controller**, at `stagger_ms(seed, id, decide_ms)` of arena time (doc 16
    /// §6 read the way §3.6's pass is: *staggered by unit id*; 16b cut U.1).
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
    /// `decide_ms` offset by `stagger_ms(seed, id, decide_ms)` that is at or after
    /// `now_ms`.
    ///
    /// The grid is a function of arena time, the match seed and the unit id alone,
    /// so a unit adopted halfway through a match lands on the same cadence as one
    /// adopted at check-out and the field stays spread.
    ///
    /// **16b cut U.1** (16c `T-25`): the offset was `id % decide_ms`, which put a
    /// field of contiguous object numbers' ticks inside the first `units`
    /// milliseconds of each decide window, the way it put the sensor passes in
    /// the first `units` milliseconds of each second. It is
    /// [`crate::arena::rng::stagger_ms`] now, the pass's own rule, and the grid
    /// formula around it is unchanged.
    fn schedule_reflex_tick(&mut self, unit: ObjId) {
        let cadence = i64::from(self.arena_cfg.reflex.decide_ms.max(1));
        let offset = crate::arena::rng::stagger_ms(self.seed, unit, cadence);
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
        // **16b cut M.3** (16c `T-04`, B-43's guarantee under the one-edge rule):
        // before the decision's `REFLEX_AVOID_TTL_MS` sweep, a machine still held
        // on an occupied cell refreshes that cell's stamp to this tick.
        self.refresh_held_avoid(unit, t_ms);
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
                Vec::new(),
                hash,
            );
            self.note_order_expired(squad);
        }
        if plan.ordered {
            counters.ordered = counters.ordered.saturating_add(1);
        }
        self.order(unit, &plan, t_ms, counters);
        true
    }

    /// **A routeless machine is asked every substep whether the cell in front of
    /// it is still open** (16b cut E.3; 16c `B-44`), before motion runs.
    ///
    /// 16b cut Q.7 put that question into `Arena::follow`: a machine steering at
    /// a goal with no route is steering at a *direction* nobody has checked the
    /// ground along, so the one cell it would cross first is tested with the
    /// crossing's own rule and a machine that would be refused it is ordered to
    /// stop. But `follow` runs at the decide tick — once a second of arena time —
    /// and motion runs eight times in that second at the fastest tempo, so for up
    /// to a whole tick the machine drove at a bluff, a wreck or the cell it was
    /// just stopped at with nothing between it and the refusal.
    ///
    /// So the question is asked at motion's own cadence. For every unit the
    /// controller is flying whose power is online, whose speed order is not
    /// already zero, and whose machine has a goal and an **empty route** — a route
    /// is a sequence of edges the search has already checked — the cell on the
    /// unit's **actual** `pose.heading_mdeg` (not the heading it was ordered onto;
    /// motion will cross the one the tank is pointing at) is put to
    /// `Arena::first_step_is_open` with the machine's own remembered stops.
    /// Where it answers `false` the unit is ordered `Speed { cms: 0 }` through
    /// [`apply_intent`], at its own current epoch, exactly the way
    /// `Arena::order` builds one: **the machine still does nothing a pilot could
    /// not.** The next decide tick orders a speed again as soon as the step on the
    /// heading it wants is open, which is Q.7's own rule unchanged.
    ///
    /// It draws nothing, reads the rows in unit-id order (the map's own), and
    /// collects the ids before it orders any of them, so no borrow is held across
    /// [`apply_intent`]. The four cheap tests come first and the map lookup last,
    /// because this runs once per substep over the whole field.
    ///
    /// # One cell was never enough: it looks as far as the unit needs to stop
    ///
    /// **16b cut M.2** (reopening E.3's `B-44` closure; the fix was the M.2
    /// review's). Asking only the one cell in front was a rule for a unit that
    /// can stop inside a cell, and a stock tank cannot: it needs `v² / 2·brake` —
    /// 56 m from 15 m/s at 2 m/s² — so the hold came when the seam was already
    /// inside its stopping distance and the tank was refused at the boundary
    /// anyway. E.3's own proof missed it only because its thirty-second window
    /// closed five hundred milliseconds before the first approach arrived; M.2's
    /// machine `STARTUP` at its tick moved the same approach one decide tick
    /// earlier and into the window.
    ///
    /// So a machine is held when **any** cell on its actual heading within
    /// `stopping_reach_cm`, beyond the one in front, would be refused by the same
    /// rule — the stopping
    /// distance from the fastest it can be going at the end of this substep, plus
    /// what it travels in the substep before the next pre-pass can ask again —
    /// walked by `Arena::way_ahead_is_open`. The one-cell question is still
    /// asked first, unchanged, so a unit at a standstill is exactly as free as it
    /// was. Held early, the tank brakes on its own profile and stops short of the
    /// seam; the next decide tick orders it on only while the cell in front is
    /// open, and the pre-pass holds it again the substep it would outrun its
    /// brakes, so it closes on the seam slower and slower and never reaches it.
    /// A platform with no braking rate is asked the one cell alone: it cannot
    /// slow down, which is `braking_speed_cms`'s answer too.
    pub fn hold_routeless(&mut self, counters: &mut ReflexCounters) {
        // The full substep, which is what the unit travels before the next
        // pre-pass: the same length `Arena::step` cuts the pulse into, read off
        // the one method both use (16b cut U.1).
        let substep = self.substep_len_ms();
        let mut holding: Vec<ObjId> = Vec::new();
        for (id, row) in self.reflex.rows() {
            if !row.route.is_empty() {
                continue;
            }
            if row.goal.is_none() {
                continue;
            }
            let Some(unit) = self.unit(*id) else {
                continue;
            };
            if unit.controller != Controller::Reflex
                || unit.power != Power::Online
                || unit.desired_speed_cms == 0
            {
                continue;
            }
            let Some(here) = self.definition.terrain.cell_of(&unit.pose) else {
                continue;
            };
            if self.first_step_is_open(unit, here, unit.pose.heading_mdeg, &row.avoid, counters)
                && self.way_ahead_is_open(unit, here, &row.avoid, stopping_reach_cm(unit, substep))
            {
                continue;
            }
            holding.push(*id);
        }
        for id in holding {
            let kind = IntentKind::Speed { cms: 0 };
            let channel = kind.channel_required();
            let intent = Intent {
                unit: id,
                channel,
                epoch: self.epochs.current(id, channel),
                kind,
                seq: 1,
            };
            counters.intents = counters.intents.saturating_add(1);
            // A speed is a **continuous** order: `apply_intent` writes it straight
            // onto the unit and never queues it, so there is nothing to take back
            // out of the pending queue the way `Arena::order` does for a discrete
            // action.
            let _ = apply_intent(self, &intent, IntentSource::Machine);
        }
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
    /// **16b cut M.3** (16c `T-04`): "once per stop" is now exact — the line and
    /// this call fire on [`crate::arena::state::UnitState::held`]'s new edge and
    /// nothing else, so a unit re-colliding with one wreck substep after substep
    /// calls this once. The stamp refresh the repeats used to give a listed cell
    /// is the tick's now (`Arena::refresh_held_avoid`), for as long as the unit is
    /// still held there.
    ///
    /// A stop of any cause throws the route away, as the flag did. A stop with
    /// [`BlockedCause::Occupied`] does two things more: it remembers the cell (see
    /// [`ReflexUnit::avoid`]) and it clears `searched_ms`, so the next tick
    /// searches at once rather than waiting out `repath_ms` against an obstacle
    /// that is not going to move. The re-search is still under
    /// `ReflexMemory::may_search`'s per-window budget, so a field that all stops
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
        remember_avoid(row, cell, t_ms);
    }

    /// **A machine still held on an occupied cell keeps it on its avoid list**
    /// (16b cut M.3; 16c `T-04` with `B-43`'s guarantee), run at the top of
    /// [`Arena::reflex_tick`], before the decision's
    /// [`REFLEX_AVOID_TTL_MS`] sweep.
    ///
    /// Until the one-edge rule, every re-collision with the wreck was a `blocked`
    /// line and a [`Arena::note_blocked`] that refreshed the cell's stamp, so a
    /// machine pinned against it never aged it out. The line now fires once per
    /// edge, and a machine cornered against a wreck for longer than the TTL would
    /// forget the one cell it is still pushing at — so the tick refreshes it off
    /// the condition instead: a `Reflex`-controlled unit whose
    /// [`crate::arena::state::UnitState::held`] names an
    /// [`BlockedCause::Occupied`] cell has that cell's entry stamped `t_ms`, or
    /// inserted under the same least-recently-stopped eviction a new stop uses.
    /// Rate-independent: the tick is a due event at its own arena millisecond, and
    /// `held` is hashed. Nothing else about the stop is re-applied — the route and
    /// `searched_ms` were the edge's to clear, once.
    fn refresh_held_avoid(&mut self, unit: ObjId, t_ms: i64) {
        let Some(held) = self
            .unit(unit)
            .filter(|state| state.controller == Controller::Reflex)
            .and_then(|state| state.held)
        else {
            return;
        };
        if held.cause != BlockedCause::Occupied {
            return;
        }
        if let Some(row) = self.reflex.units.get_mut(&unit) {
            remember_avoid(row, (held.x, held.y), t_ms);
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
    ///
    /// **16b cut E.1 (16c `T-32`)**: the reach is
    /// [`UnitState::fight_inner_cm`] — the band the platform would actually
    /// fight in — and not the longest gun it carries. Against the 3 km cannon
    /// this answered `true` for *any* classified hostile a sight or a mesh hop
    /// could hold, so `Engaged` meant "somebody holds a classified hostile" and
    /// the arena ran at 500 ms from the first classification to the end of the
    /// match. It now means what doc 16 §3.1's table says: hostiles are within
    /// weapons range of each other.
    ///
    /// **16b cut E.2 (16c `B-45`)**: and *hostile* is read off the contact,
    /// through [`Arena::contact_reads_dead`], rather than off the target's live
    /// wreck flag. The tempo now falls when the platforms holding the wreck
    /// **learn** it — at the kill for whoever fired the round, at the track's
    /// expiry for everybody else — and not at the arena millisecond the hull
    /// burned. A contact whose unit has left the match still counts for nothing.
    #[must_use]
    pub fn hostile_in_weapons_range(&self) -> bool {
        let cell_cm = self.definition.terrain.cell_cm();
        self.units.iter().any(|viewer| {
            let reach = viewer.fight_inner_cm(cell_cm);
            reach != 0
                && self.contacts.of(viewer.id).any(|(_, held)| {
                    held.class.team().is_some_and(|team| team != viewer.team)
                        && self.unit(held.target).is_some()
                        && !self.contact_reads_dead(held)
                        // 16b cut U.1 (16c `T-24`): the squared test, no root.
                        && held.seen_at.within_cm(&viewer.pose, reach)
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
        // list in exactly the case it exists for. **16b cut M.3**: a cell the
        // unit is still held against was re-stamped to this tick just before
        // (`Arena::refresh_held_avoid`), so it survives this sweep however long
        // the unit has been pinned there.
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
        // **16b cut AA.1 (16c `B-76`, the order layer)**: an `Advance` held by a
        // machine that cannot close is an order it can never carry out, and one it
        // would carry for its whole TTL — the ordered cell is reachable, so
        // `B-41`'s unreachable arm never fires; the hostile edges are suppressed
        // under it (16b cut N.2); and the squad signal carries no mobility fact, so
        // the commander cannot learn why nothing moves. It ends for THIS unit here,
        // through the same `plan.expired` `B-41` uses: `Arena::reflex_tick`
        // journals the one `order_expired` line, the squad's next signal says
        // `OrderExpired`, and from this tick the unit is on doctrine and fights what
        // it holds. `Hold` is left alone: a machine under `Hold` does not fight
        // whether or not it can move, and that is the commander's choice.
        let order = match order {
            Some(order)
                if matches!(order.objective, Objective::Advance { .. }) && unit.cannot_close() =>
            {
                row.order = None;
                plan.expired = Some((order.squad, order.objective.as_str()));
                None
            }
            other => other,
        };

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
        //
        // **16b cut E.2 (16c `B-45`)**: and a target the platform **knows** is
        // dead — its own kill, or a reading that shows the body gone
        // ([`Arena::contact_reads_dead`]). Before this cut the edge read the
        // target's live `limits.wreck`, so a machine dropped a target the pulse
        // it died however stale its track, whoever killed it, through a wall and
        // across the map. That is hidden truth steering a unit, which doc 16 §1
        // forbids every other reader; a pilot in the same seat keeps the row on
        // `CONTACTS` until it expires, and so does the machine now.
        let held_target = row
            .target
            .and_then(|(contact, _)| self.contacts.resolve(id, contact));
        if let Some((_, target)) = row.target
            && (held_target.map(|held| held.target) != Some(target)
                || self.unit(target).is_none()
                || held_target.is_some_and(|held| self.contact_reads_dead(held)))
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
        //   two (an `advance` held by a machine that cannot close never reaches
        //   here: it ended above the power arms, 16b cut AA.1);
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
                        // **16b cut E.4**: a machine breaking off does not search.
                        // Its goal is its own entry zone, and the last place it
                        // saw the enemy is the one place it must not drive to.
                        row.last_plot = None;
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
            // **16b cut E.4**: `Withdraw` does not search — see the order's own
            // arm above for why.
            row.last_plot = None;
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
        // **16b cut E.1**: the three distances this tick fights at, computed once
        // from the unit, the doctrine and the order — see [`Ranges`] for what
        // each one is and why the doctrine band is applied after the order's cap.
        let cell_cm = self.definition.terrain.cell_cm();
        let ranges = ranges(unit, cell_cm, &memory.cfg, order.as_ref());
        // **16b cut E.4**: the cell a `Withdraw` that stood and fought turned
        // onto. `Withdraw` clears `row.target` on entry and `withdraw_stand`
        // chooses its own candidate without writing one back, so the stand is the
        // one plot this tick holds that `row.target` cannot name.
        let mut stood_cell: Option<(u16, u16)> = None;
        match row.state {
            ReflexState::Startup | ReflexState::Disabled => {}
            ReflexState::Patrol => {
                // 16b cut N.2: `advance` replaces the waypoint walk with the cell
                // the commander named; `hold` parks where the unit stands. Both
                // leave the waypoint the machine was walking to alone, so doctrine
                // picks it up again at expiry rather than starting from nothing.
                //
                // **16b cut E.4**: but neither of the endgame's two clocks. A
                // movement order is the head of `Patrol`'s list, and a commander
                // who said *go there* is not answered at the order's expiry by a
                // machine turning round for a plot it saw before the order, or
                // finishing a crossing it had started before it: both goals are
                // dropped on every ordered tick, and the hunt's wait restarts on
                // each, so it is counted from the order's end and not through it
                // (the order suppresses the hostile edges, so what the squad saw
                // while it obeyed was never read by the clock anyway).
                let movement = matches!(
                    order.as_ref().map(|order| order.objective),
                    Some(Objective::Advance { .. } | Objective::Hold)
                );
                if movement {
                    row.last_plot = None;
                    row.hunt = None;
                    row.unheld_since_ms = Some(t_ms);
                }
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
                    // **16b cut E.4 (16c `B-04`)**: with no movement order the
                    // goal is the search, then the hunt, then the waypoint walk.
                    // Each answers `true` when it has taken the goal, so the one
                    // below it is not asked; a movement order above never reaches
                    // here, and has just cleared both clocks' goals, so neither
                    // survives one.
                    _ => {
                        if !self.search_goal(t_ms, &mut row, here, &memory.cfg)
                            && !self.hunt_goal(id, t_ms, &mut row, here, &memory.cfg)
                        {
                            self.patrol_goal(id, &mut row, here);
                        }
                    }
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
            //
            // **16b cut E.1** gave this arm three cases instead of two, and
            // priced all of them off [`Ranges`]:
            //
            // * **inside `engage_cm` with a line** — `Engage`, and
            //   `engage_orders` on this same tick. The first shot is therefore
            //   fired *still rolling*: `speed 0` is an order and a stock tank
            //   takes 7.5 s to obey it. That is deliberate — a crew with a line
            //   shoots — and it is why the duel test asserts the **ordered**
            //   speed at the first shot and not the actual one;
            // * **inside `engage_cm` with no line** — the goal is the plot's own
            //   cell (16c's Q9: *it closes on the plot*), and the tick after it
            //   has moved the trace is taken again;
            // * **outside `engage_cm`** — the stand-off cell, and **no trace**.
            //   Before this cut a trace was paid at every range, including the
            //   ones at which nothing could be shot at anyway.
            //
            // **16b cut AA.1 (16c `B-76`)** moved the edge the three cases are
            // split at for one kind of machine:
            //
            // * **a machine that cannot close** ([`UnitState::cannot_close`]) —
            //   the edge is `release_cm` and not `engage_cm`, so it enters
            //   `Engage` wherever it stands inside the reach it may fire across.
            //   Before this cut a tank whose engine was killed between the two
            //   edges re-planned a stand-off cell it could not drive to on every
            //   tick and never fired again, and being armed it held its side up
            //   for the rest of the match.
            ReflexState::Approach => {
                if let Some((contact, target)) = row.target {
                    let plot = self.plot_of(id, contact);
                    let range = plot.map_or(u64::MAX, |plot| plot.range_from(&unit.pose));
                    // **16b cut AA.1 (16c `B-76`)**: a machine that cannot close
                    // has nowhere to close to. Its edge into `Engage` is the whole
                    // of the reach it may fire across, `release_cm` — the same
                    // ceiling `engage_orders` holds and the same edge `Engage`
                    // gives the band up at, so it can neither stand outside a
                    // range it could shoot from nor oscillate. A machine that CAN
                    // close still stops at `engage_cm` and holds its fire through
                    // the outer three tenths: firing on the move while closing is
                    // not built (16b row E).
                    let close_edge = if unit.cannot_close() {
                        ranges.release_cm
                    } else {
                        ranges.engage_cm
                    };
                    if ranges.limit_cm != 0 && range <= close_edge.max(1) {
                        if self.reflex_has_line(id, target, counters) {
                            row.state = ReflexState::Engage;
                            row.goal = None;
                            row.route.clear();
                            self.engage_orders(
                                id,
                                contact,
                                order.as_ref(),
                                &ranges,
                                t_ms,
                                &mut plan,
                            );
                        } else {
                            row.goal = plot
                                .and_then(|plot| plot.cell(&self.definition.terrain))
                                .or(here);
                        }
                    } else {
                        let want =
                            stand_off_want(ranges.engage_cm, u64::from(cell_cm.max(1)), range);
                        row.goal = plot
                            .and_then(|plot| self.stand_off_cell(id, &plot, want))
                            .or(here);
                    }
                }
            }
            ReflexState::Engage => {
                if let Some((contact, _)) = row.target {
                    let plot = self.plot_of(id, contact);
                    let range = plot.map_or(u64::MAX, |plot| plot.range_from(&unit.pose));
                    // **16b cut E.1**: the edge out is `release_cm`, the whole of
                    // the allowed reach, and `engage_cm` is seven tenths of it —
                    // so there is three tenths of hysteresis between closing and
                    // holding, under every order. Before this cut both edges read
                    // one number and neither could ever fire (16c `T-47`).
                    if ranges.limit_cm == 0 || range > ranges.release_cm {
                        row.state = ReflexState::Approach;
                        let want =
                            stand_off_want(ranges.engage_cm, u64::from(cell_cm.max(1)), range);
                        row.goal = plot
                            .and_then(|plot| self.stand_off_cell(id, &plot, want))
                            .or(here);
                    } else {
                        // Holding the band: stop, turn onto the plot, shoot.
                        row.goal = here;
                        row.route.clear();
                        plan.speed_cms = Some(0);
                        plan.heading_mdeg = plot.map(|plot| plot.bearing_from(&unit.pose));
                        self.engage_orders(id, contact, order.as_ref(), &ranges, t_ms, &mut plan);
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
                // **16b cut E.1**: whether it is home, computed once here and
                // read twice — the stand's ceiling is `release_cm` once it has
                // arrived and `stand_cm` while it is still running, and the
                // rally below asks the same question.
                //
                // **16b cut AA.1 (16c `B-76`)**: a machine that cannot move is
                // home wherever it is — the argument `withdraw_stand`'s doc
                // makes for a unit with no goal at all ("there is no home to run
                // to, which is the arrived argument exactly") holds word for
                // word for one that cannot reach it. Both readers take it, on
                // purpose: the stand's ceiling is `release_cm` at any health, so
                // a hurt immobile tank shoots back at anything with a line inside
                // it instead of being parked in the `close` band; and the rally
                // clock runs for it once nothing hostile is held, so it rallies
                // once like a machine that is home and then fights to the end.
                let arrived = unit.cannot_close()
                    || match (here, row.goal) {
                        (Some(here), Some(goal)) => {
                            here.0.abs_diff(goal.0).max(here.1.abs_diff(goal.1)) <= ARRIVE_CELLS
                        }
                        (_, None) => true,
                        _ => false,
                    };
                // (a) **cornered**. A hostile it holds, classified, inside the
                // reach its state and its order allow it, with a line: it stops
                // where it is, turns onto the plot and shoots — **without leaving
                // `Withdraw` and without touching the goal or the route**, so the
                // run home resumes from where it stopped rather than from a fresh
                // search. A tank with nothing to fire, or one told to hold its
                // fire, is never cornered: see [`Arena::withdraw_stand`].
                let cornered = self.withdraw_stand(
                    id,
                    memory,
                    health,
                    arrived,
                    &ranges,
                    order.as_ref(),
                    counters,
                );
                if let Some((contact, plot)) = cornered {
                    stood_cell = plot.cell(&self.definition.terrain);
                    row.stand_until_ms = Some(t_ms.saturating_add(
                        REFLEX_STAND_TICKS.saturating_mul(i64::from(memory.cfg.decide_ms)),
                    ));
                    row.quiet_since_ms = None;
                    plan.speed_cms = Some(0);
                    plan.heading_mdeg = Some(plot.bearing_from(&unit.pose));
                    self.engage_orders(id, contact, order.as_ref(), &ranges, t_ms, &mut plan);
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
                    //
                    // **16b cut E.2 (16c `B-40`)**: and neither does a unit a
                    // commander has told to break off. The rally is doctrine's
                    // own answer to "the board is quiet"; under a live
                    // `Objective::Withdraw` the arm at the top of this function
                    // puts the machine straight back into `Withdraw` on the very
                    // next tick and throws its route away with it, so what the
                    // rally bought was one lost route per `rally_ms` for the
                    // order's whole life. The clock is not merely ignored, it is
                    // held at `None`: see [`ReflexUnit::quiet_since_ms`]. When
                    // the order expires the expiry block above returns a healthy
                    // unit to `Patrol` at once, and a hurt one starts its quiet
                    // from that tick.
                    let ordered_home = matches!(
                        order.as_ref().map(|order| order.objective),
                        Some(Objective::Withdraw)
                    );
                    if self.holds_a_hostile(id) || !arrived || dry || ordered_home {
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

        // ---- what this tick saw, and how long it has seen nothing (16b cut E.4)
        //      ----
        //
        // The search's memory is written **last**, from the plot this tick
        // actually acted on: `Approach` and `Engage` write the plot of their own
        // target, and a `Withdraw` that stopped and fought writes the plot it
        // turned onto. It is the cell of the **plot** and not of the target, so a
        // tank that drives on after the contact expires does not drag the search
        // after it (doc 16 §1); a plot that is off the window writes nothing,
        // because there is no cell to drive to.
        if let Some(cell) = stood_cell {
            row.last_plot = Some((cell, t_ms));
        } else if matches!(row.state, ReflexState::Approach | ReflexState::Engage)
            && let Some((contact, _)) = row.target
            && let Some(cell) = self
                .plot_of(id, contact)
                .and_then(|plot| plot.cell(&self.definition.terrain))
        {
            row.last_plot = Some((cell, t_ms));
        }
        // The hunt belongs to `Patrol` and to nothing else: a machine that has
        // left it — for a fight, for home, for an order — gives its crossing up
        // and starts the wait again from here, so it is a minute and a half of
        // *nothing at all* that sends a machine over and never a minute and a
        // half measured through a battle.
        if row.state != ReflexState::Patrol {
            row.hunt = None;
            row.unheld_since_ms = Some(t_ms);
        }

        // ---- the route, and the steer ----
        if !matches!(row.state, ReflexState::Startup | ReflexState::Disabled)
            && plan.speed_cms.is_none()
        {
            // **16b cut E.2 (16c `B-41`)**: the steer is handed the order this
            // tick decided under — the one the squad filter above already
            // applied — because a search that fails for the order's OWN cell
            // ends the order for this unit, and nothing else does.
            self.follow(
                id,
                t_ms,
                &mut row,
                here,
                memory,
                counters,
                order.as_ref(),
                &mut plan,
            );
        }
        memory.units.insert(id, row);
        plan
    }

    /// **`Patrol`'s first goal: where the machine last saw something** (16b cut
    /// E.4; 16c `B-04`). Answers whether it took the goal.
    ///
    /// A contact expires after three sensor passes and the `lost` edge drops the
    /// machine here. Before this cut the waypoint walk took it wherever the
    /// graph's fixed cycle said, which may be directly away from the tank that
    /// has just driven behind a tree line; now it drives to
    /// [`ReflexUnit::last_plot`] for [`crate::config::ReflexConfig::search_ms`]
    /// of arena time first.
    ///
    /// The three ways it ends are all clears of `last_plot`, so a machine whose
    /// search is over asks this once more and is answered `false`: **arrival**
    /// within [`ARRIVE_CELLS`] (it went and looked and there was nothing there),
    /// **age** past `search_ms` (an age of exactly `search_ms` still searches —
    /// see the comparison below for why), and the key at **`0`**, which
    /// reproduces the pre-E.4 `lost` edge exactly. The fourth is the
    /// `unreachable` edge, in [`Arena::follow`], and a movement order clears it
    /// too, in `decide`'s `Patrol` arm.
    ///
    /// A hostile found on the way pre-empts it without this being asked at all:
    /// the hostile edges run above the state's own body and put the machine in
    /// `Approach`.
    fn search_goal(
        &self,
        t_ms: i64,
        row: &mut ReflexUnit,
        here: Option<(u16, u16)>,
        cfg: &ReflexConfig,
    ) -> bool {
        let Some((cell, at_ms)) = row.last_plot else {
            return false;
        };
        // The age is compared **inclusively**: the plot was written by the last
        // tick that held the target and the `lost` edge is read one decide tick
        // later, so the first `Patrol` tick always finds it exactly `decide_ms`
        // old. At the validated floor (`search_ms == decide_ms`) a strict
        // comparison would drop it on that tick and the key would be `0` under
        // another name; inclusively the machine drives at it for that one tick.
        if cfg.search_ms == 0 || t_ms.saturating_sub(at_ms) > i64::from(cfg.search_ms) {
            row.last_plot = None;
            return false;
        }
        if here.is_some_and(|here| {
            here.0.abs_diff(cell.0).max(here.1.abs_diff(cell.1)) <= ARRIVE_CELLS
        }) {
            row.last_plot = None;
            return false;
        }
        if row.goal != Some(cell) {
            row.goal = Some(cell);
            row.route.clear();
        }
        true
    }

    /// **`Patrol`'s second goal: a machine that has seen nobody goes and looks**
    /// (16b cut E.4; 16c `B-04`). Answers whether it took the goal.
    ///
    /// The waypoint walk is `choices[id % choices.len()]`, a deterministic loop
    /// per unit id on a graph of two roads and four spurs, so two sides' loops
    /// need never come within a sight of each other — and a withdrawn machine
    /// parks in its own entry zone, which no loop passes through except at the
    /// two road ends. Measured at HEAD before this cut: a dry Red tank on its
    /// entry cell from 30 s, a Blue tank walking the east half of the map, and
    /// nothing at all for the next fifteen minutes of arena time.
    ///
    /// So a machine that has held nothing hostile at all
    /// ([`Arena::holds_a_hostile`], which is E.2's reading: a contact that reads
    /// dead is not a hostile) for
    /// [`crate::config::ReflexConfig::hunt_after_ms`] crosses to the **other**
    /// side's entry zone, which is the one place on the map a beaten enemy is
    /// certain to be. It reads [`Arena::far_entry_cell`], which is the arena
    /// definition's own zones and no unit's pose at all, so the machine has been
    /// told nothing about anybody.
    ///
    /// **It is not staggered by unit id, on purpose.** Every machine of a side
    /// starts its clock within one decide period of the others, so a side that
    /// has seen nobody sets out within a second of itself — which is wanted: a
    /// side that crosses together arrives together, and one that trickled across
    /// by id would be met one at a time. What keeps it from being a traffic jam
    /// is already here: [`Arena::far_entry_cell`]'s clamp keeps each machine on
    /// its own row, [`ReflexMemory::may_search`] rations the searches, and 16b
    /// cut E.3's hold is what a column at a ford needs.
    ///
    /// A far zone that answers `None` — no other side has one, or nothing in it
    /// is passable — restarts the clock rather than leaving it run out, so the
    /// scan is paid once per `hunt_after_ms` and not once per tick. That is the
    /// honest cost of an answer that is honestly nothing.
    fn hunt_goal(
        &self,
        id: ObjId,
        t_ms: i64,
        row: &mut ReflexUnit,
        here: Option<(u16, u16)>,
        cfg: &ReflexConfig,
    ) -> bool {
        if self.holds_a_hostile(id) {
            row.unheld_since_ms = None;
            row.hunt = None;
            return false;
        }
        if let Some(cell) = row.hunt {
            if here.is_some_and(|here| {
                here.0.abs_diff(cell.0).max(here.1.abs_diff(cell.1)) <= ARRIVE_CELLS
            }) {
                // It went and looked. The walk resumes from the nearest node —
                // which at the far edge is the enemy's own end of the road — and
                // the next hunt, `hunt_after_ms` later, starts from wherever that
                // walk has taken it, so successive crossings sweep different rows
                // of the zone.
                //
                // The waypoint it was walking to before it crossed is forgotten
                // **here**: [`Arena::patrol_goal`] keeps a node it holds and asks
                // the graph for the nearest only when it holds none, so without
                // this the machine would turn round at the far edge and drive the
                // whole map back to a node on its own side.
                row.hunt = None;
                row.unheld_since_ms = Some(t_ms);
                row.waypoint = None;
                row.from_waypoint = None;
                return false;
            }
            if row.goal != Some(cell) {
                row.goal = Some(cell);
                row.route.clear();
            }
            return true;
        }
        let since = *row.unheld_since_ms.get_or_insert(t_ms);
        if cfg.hunt_after_ms == 0 || t_ms.saturating_sub(since) < i64::from(cfg.hunt_after_ms) {
            return false;
        }
        let Some(cell) = self.far_entry_cell(id) else {
            row.unheld_since_ms = Some(t_ms);
            return false;
        };
        row.hunt = Some(cell);
        if row.goal != Some(cell) {
            row.goal = Some(cell);
            row.route.clear();
        }
        true
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
    ///
    /// **16b cut M.2** (16c `T-02`): readiness is read at `t_ms`, the tick's own
    /// arena millisecond, which is the millisecond [`Arena::resolve_fire`] is then
    /// handed. It read `now_ms` — the end of the substep the tick ran in — so a
    /// weapon coming ready between the tick and the substep end was planned as a
    /// shot, refused `Cycling` by a resolver reading the tick's clock, and the
    /// refusal (which `Arena::order` drops) cost the machine a whole `decide_ms`.
    fn engage_orders(
        &self,
        id: ObjId,
        contact: ContactId,
        order: Option<&Order>,
        ranges: &Ranges,
        t_ms: i64,
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
        // **16b cut E.1**: the firing ceiling is `Ranges::release_cm`, the same
        // number `Engage` holds out to — so a machine can fire across the whole
        // of its `Engage` region and never stands in it at a range it may not
        // shoot from.
        if range > ranges.release_cm {
            return;
        }
        // **16b cut Q.7**: and the heat. The controller reads the platform's
        // ceiling the way `crate::arena::weapons`'s acceptance does — the `heat`
        // cell against its own maximum — so it does not feed an `Overheated`
        // refusal into the resolver every tick and then sit with a hot gun.
        //
        // **16b cut E.2 (16c `T-55`)**: and it is the engine's line and not one
        // shot short of it. The engine refuses AT the ceiling and accepts and
        // clamps below it, so the machine takes a hot shot that will land it on
        // the ceiling and holds its fire on the next tick — where before it
        // declined the shot a pilot in the same seat could fire. A unit whose
        // content declares no heat is never held back here, which is every stock
        // tank. See [`would_overheat`].
        plan.fire = unit
            .weapons
            .iter()
            .find(|weapon| {
                unit.weapon_ready(&weapon.id, t_ms)
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
    /// tank it would have closed on — and the tests after it are `Approach`'s
    /// own, in `Approach`'s own order: the **plot's** range against a ceiling
    /// (16b cut F.3: the range the gunner has), and then one trace. The trace is
    /// paid only when the range test has already passed, so a withdrawing tank
    /// with nothing near it costs a `Withdraw` tick exactly what it cost before
    /// this cut.
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
    ///
    /// # The ceiling is two rules, and `arrived` is which (16b cut E.1, 16c
    /// `B-39`, 16c §5's Q8)
    ///
    /// Before this cut the ceiling was the platform's whole 3 km reach, which on
    /// this map is always true: a hurt machine stood and fought whatever it could
    /// see, at any health, and never got home.
    ///
    /// * **en route** — the plot must be inside [`Ranges::stand_cm`], the `close`
    ///   band, **and** the unit must have at least
    ///   [`crate::config::ReflexConfig::stand_min_permille`] of its armour and
    ///   internals left. It is running home; it turns for something on top of it
    ///   and for nothing else, and not once it is down to the last of its plate.
    /// * **arrived** — the plot must be inside [`Ranges::release_cm`], **at any
    ///   health**. A machine that is home has nowhere further to run, and one
    ///   that is shot at 420 m and never replies is the parked target 16b cut Q.7
    ///   was paid to remove.
    ///
    /// **A unit with no goal at all counts as arrived, on purpose.** The rally's
    /// own test answers `true` for `(_, None)` and `Withdraw`'s goal is
    /// `entry_cell(id).or(here)`, so `None` means *this arena gave my side no
    /// entry zone and I am not on the window*: there is no home to run to, which
    /// is the arrived argument exactly, and the machine shoots back at whatever
    /// comes inside `release_cm`.
    ///
    /// **A unit that cannot close counts as arrived, on the same argument** (16b
    /// cut AA.1; 16c `B-76`). A destroyed engine, or a hull authored with no
    /// speed ([`UnitState::cannot_close`]), has a home it cannot reach, which is
    /// no home to run to at all: `decide` passes `arrived` as `true` for it
    /// wherever it stands, and it shoots back inside `release_cm` at any health
    /// rather than being parked in front of a gun it will not answer.
    #[allow(clippy::too_many_arguments)]
    fn withdraw_stand(
        &self,
        id: ObjId,
        memory: &ReflexMemory,
        health_permille: u32,
        arrived: bool,
        ranges: &Ranges,
        order: Option<&Order>,
        counters: &mut ReflexCounters,
    ) -> Option<(ContactId, SeenPose)> {
        let unit = self.unit(id)?;
        if is_dry(unit) || order.is_some_and(|order| order.constraints.hold_fire) {
            return None;
        }
        let (contact, target) = self.best_hostile(id, memory, health_permille, None)?;
        let plot = self.plot_of(id, contact)?;
        let ceiling = if arrived {
            ranges.release_cm
        } else {
            ranges.stand_cm
        };
        if plot.range_from(&unit.pose) > ceiling.max(1) {
            return None;
        }
        if !arrived && health_permille < memory.cfg.stand_min_permille {
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
    ///
    /// **16b cut E.2 (16c `B-45`)**: *live* is [`Arena::contact_reads_dead`] and
    /// not the target's own wreck flag, so a machine that killed the last
    /// hostile starts its quiet clock at once and one that watched a teammate do
    /// it out of its own sight waits for the track to expire. A contact whose
    /// unit has left the match is not a hostile either way.
    fn holds_a_hostile(&self, id: ObjId) -> bool {
        self.unit(id).is_some_and(|unit| {
            self.contacts.of(id).any(|(_, held)| {
                matches!(held.class, Classification::Classified { team } if team != unit.team)
                    && self.unit(held.target).is_some()
                    && !self.contact_reads_dead(held)
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
    ///
    /// **16b cut E.2 (16c `B-45`) deliberately left this alone**, with
    /// [`Arena::fire_line`], the sensor pass's own `sees_wrecks` rule,
    /// `Arena::resolve_lock`, `Arena::resolve_fire` and the impact's
    /// re-validation. Those are **physics**: they answer *would this round
    /// arrive*, which is not a thing the shooter knows in advance. A machine
    /// that fires at a wreck it still believes in spends the round and reads a
    /// miss, as a pilot would.
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
        //
        // **16b cut E.1**: the reach is [`UnitState::fight_limit_cm`], so the
        // rule now also covers "every magazine is empty" — which [`is_dry`] sends
        // to `Withdraw` first anyway — and the range term below is a fraction of
        // the distance this platform really fights at rather than of a 3 km gun
        // nothing on the map is ever held at.
        let reach = unit.fight_limit_cm(self.definition.terrain.cell_cm());
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
            // **16b cut E.2** (16c `B-45`): a candidate this platform **knows**
            // is dead is no candidate. Read off the hold and not off
            // `target.limits.wreck`, so the score ranks what the machine holds
            // rather than what the world is; a track that has not caught up yet
            // still ranks, and the machine spends a round finding out, exactly
            // as a gunner would.
            if self.contact_reads_dead(held) {
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

    /// The cell to stand in to hold `want_cm` of range on `at` — the goal
    /// `Approach` drives to.
    ///
    /// The point on the line from the plot toward *here*, at the holding range.
    /// A cell off the window or impassable falls back to the plot's own cell,
    /// and the path search's `unreachable` edge is what catches that.
    ///
    /// **16b cut F.3** made `at` the contact's plot rather than the target's
    /// object: an approach is toward the last known position, and a unit that
    /// arrives there and finds nothing is a unit that has lost its contact.
    ///
    /// **16b cut E.1**: `want_cm` comes from [`stand_off_want`], which is what
    /// guarantees `want_cm ≤ range` — and that is what guarantees the **point**
    /// this projects is never further from the plot than the unit already
    /// stands, so the goal is never behind the machine (16c `B-05`). The *cell*
    /// it is quantised to may be up to seven tenths of a cell further out in the
    /// degenerate case where the floor of one cell is the answer, and is two
    /// cells nearer or better at every shipped distance.
    fn stand_off_cell(&self, id: ObjId, at: &SeenPose, want_cm: u64) -> Option<(u16, u16)> {
        let from = self.unit(id)?;
        let range = at.range_from(&from.pose);
        let want = i64::try_from(want_cm.max(1)).unwrap_or(i64::MAX);
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

    /// **The hunt's goal: a cell of some OTHER side's entry zone** (16b cut E.4;
    /// 16c `B-04`), or `None`.
    ///
    /// [`Arena::entry_cell`] is deliberately **not** touched by this cut — not
    /// its zone choice, not its metric, not its fallback — so `Withdraw`'s goal
    /// is exactly what it was. This is a second function with its own rule, and
    /// the whole of that rule is here:
    ///
    /// * over the entry zones whose team is **not** this unit's, **in zone
    ///   order**;
    /// * each zone's candidate is its **clamped corner** — the unit's own cell
    ///   clamped into the zone — when that cell is passable, else the **nearest
    ///   passable cell of the zone by Manhattan distance**, which is the metric
    ///   [`Arena::entry_cell`]'s scan already ranks by, so the file has one
    ///   ranking and not two;
    /// * the scan reads the zone's rows **nearest the unit's own row first** —
    ///   the clamped row, then one either side, then two — because
    ///   [`MAX_ENTRY_SCAN`] is 1,024 cells and a shipped zone is 16 by 256, so a
    ///   row-major scan from `y0` would hand a machine standing on row 200 a
    ///   cell on row 60;
    /// * one budget of [`MAX_ENTRY_SCAN`] cells is shared across every zone;
    /// * the answer is the candidate **nearest the unit** by that same distance,
    ///   ties to the **lower zone index**.
    ///
    /// **It never answers a cell the passability test rejected.**
    /// [`Arena::entry_cell`] ends `.or(Some(clamped))`, which can hand back the
    /// impassable corner it has just refused; for a `Withdraw` that is a goal the
    /// search fails on and a fallback waypoint catches, but for a hunt it would
    /// be an unreachable cell re-asserted every `hunt_after_ms` for the rest of
    /// the match. When the budget runs out with nothing found, or no other side
    /// has a zone at all, this answers `None` and the hunt does nothing — which
    /// is the honest answer, and [`Arena::hunt_goal`] restarts its clock on it so
    /// the scan is paid once per `hunt_after_ms`.
    fn far_entry_cell(&self, id: ObjId) -> Option<(u16, u16)> {
        let unit = self.unit(id)?;
        let (here_x, here_y) = self.definition.terrain.cell_of(&unit.pose)?;
        let passable = |x: u16, y: u16| {
            self.definition
                .terrain
                .get(x, y)
                .is_some_and(crate::arena::state::TerrainCell::passable)
        };
        let reach = |x: u16, y: u16| u32::from(x.abs_diff(here_x)) + u32::from(y.abs_diff(here_y));
        let mut best: Option<(u32, (u16, u16))> = None;
        let mut read = 0_usize;
        for zone in self
            .definition
            .entry_zones
            .iter()
            .filter(|zone| zone.team != unit.team)
        {
            let (x0, x1) = (zone.x0.min(zone.x1), zone.x0.max(zone.x1));
            let (y0, y1) = (zone.y0.min(zone.y1), zone.y0.max(zone.y1));
            let (cx, cy) = (here_x.clamp(x0, x1), here_y.clamp(y0, y1));
            if passable(cx, cy) {
                let distance = reach(cx, cy);
                if best.is_none_or(|(far, _)| distance < far) {
                    best = Some((distance, (cx, cy)));
                }
                continue;
            }
            // The rows nearest the unit's own first: `cy`, then `cy - 1` and
            // `cy + 1`, then two, and so on to the ends of the zone.
            let span = u32::from(y1.saturating_sub(y0));
            'zone: for offset in 0..=span {
                for below in [true, false] {
                    if !below && offset == 0 {
                        // The clamped row is read once, not twice.
                        continue;
                    }
                    let at = if below {
                        u32::from(cy).checked_sub(offset)
                    } else {
                        u32::from(cy).checked_add(offset)
                    };
                    let Some(Ok(y)) = at.map(u16::try_from) else {
                        continue;
                    };
                    if y < y0 || y > y1 {
                        continue;
                    }
                    for x in x0..=x1 {
                        read = read.saturating_add(1);
                        if read > MAX_ENTRY_SCAN {
                            break 'zone;
                        }
                        if !passable(x, y) {
                            continue;
                        }
                        let distance = reach(x, y);
                        if best.is_none_or(|(far, _)| distance < far) {
                            best = Some((distance, (x, y)));
                        }
                    }
                }
            }
            if read > MAX_ENTRY_SCAN {
                break;
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
    ///
    /// `order` is the standing order **this tick decided under** — already
    /// filtered by [`Arena::decide`]'s squad test — and it is here for exactly
    /// one thing: 16b cut E.2's `B-41` rule, that a failed search for the
    /// order's own `Advance` cell ends the order for this unit. It is compared
    /// against rather than `row.order` so that a unit whose roster moved under
    /// the order, and which is therefore obeying nothing, drops nothing either.
    #[allow(clippy::too_many_arguments)]
    fn follow(
        &self,
        id: ObjId,
        t_ms: i64,
        row: &mut ReflexUnit,
        here: Option<(u16, u16)>,
        memory: &mut ReflexMemory,
        counters: &mut ReflexCounters,
        order: Option<&Order>,
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
                let scale = self.min_move_cost.max(1);
                // **16b cut Q.7**: the cells this unit has been stopped at, at
                // most `REFLEX_AVOID_MAX` of them, built once per search.
                let avoid: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
                // **16b cut X.6**: the unit's own slope limit goes into the
                // search, so the route the controller follows is a route the
                // crossing will accept. A route over an edge motion then refuses
                // is a tank driving at a bluff and re-planning at it every
                // `repath_ms` for the rest of the match.
                let searched = path_with(
                    &self.definition.terrain,
                    &mut memory.scratch,
                    here,
                    goal,
                    budget,
                    scale,
                    unit.slope_limit_pct,
                    &avoid,
                );
                // 16b cut U.1: what the search refused by slope, won or lost.
                counters.slope_refusals = counters
                    .slope_refusals
                    .saturating_add(memory.scratch.slope_refusals());
                match searched {
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
                        // **16b cut E.2 (16c `B-41`)**: and when the goal that
                        // failed is the live order's own `Advance` cell, the
                        // **order is over for this unit**.
                        //
                        // Before this cut the fallback below took, and the
                        // `Patrol` arm re-asserted the ordered cell on the very
                        // next tick (`row.goal != Some(to)`), so the fallback
                        // never held: the unit steered straight at ground no
                        // route reaches and paid a failed search every
                        // `repath_ms` for the order's whole life — and because
                        // an `Advance` suppresses the hostile edges, a squad
                        // sent to an unreachable cell was a pacifist patrol for
                        // up to `MAX_TTL_MS`. The only channel back to the
                        // commander, `Arena::note_order_expired`, fired on
                        // expiry alone, so nobody was ever told.
                        //
                        // Nothing more is needed here: `Arena::reflex_tick`
                        // already turns `plan.expired` into the one
                        // `order_expired` line (this unit, its squad, the word
                        // `advance`), counts it in
                        // `ReflexCounters::orders_expired` and calls
                        // `note_order_expired`, so the squad's next signal
                        // carries `SignalReason::OrderExpired` and the commander
                        // decides again — no sooner than `deliberate_min_ms`
                        // after its last signal. From the next tick the unit is
                        // on doctrine: the fallback waypoint stands because
                        // nothing re-asserts the cell, the hostile edges run
                        // again, and it fights what it meets.
                        //
                        // This is one of the two places an `Advance` ends for
                        // one unit; the other is at the top of `decide`, for a
                        // machine that cannot close (16b cut AA.1), whose search
                        // to the ordered cell succeeds and never gets here.
                        //
                        // **Only the ordered cell.** A stand-off cell under an
                        // `Engage` order, the entry cell under a `Withdraw` and
                        // a patrol node all fail through this same arm and drop
                        // nothing: they fall back exactly as they did.
                        //
                        // What it costs, stated: a commander that re-sends the
                        // same unreachable cell buys one failed search per unit
                        // per order, which the deliberation floor bounds at one
                        // per `deliberate_min_ms`; and a unit whose route was
                        // sealed only by a remembered stop
                        // ([`ReflexUnit::avoid`], which ages out after
                        // [`REFLEX_AVOID_TTL_MS`]) loses the rest of that order
                        // where a retry thirty seconds later would have kept it.
                        // The commander is told within one signal either way,
                        // which is the better half of that trade.
                        if let Some(order) = order
                            && let Objective::Advance { to } = order.objective
                            && row.goal == Some(to)
                        {
                            row.order = None;
                            plan.expired = Some((order.squad, order.objective.as_str()));
                        }
                        // **16b cut E.4 (16c `B-04`, and `B-41`'s lesson applied
                        // twice more)**: a search goal or a hunt goal that no
                        // route reaches is given up **here**, beside the order's,
                        // or the `Patrol` arm would re-assert it on the very next
                        // tick and the fallback waypoint would never hold — a
                        // plot sealed inside a building, or an island cell of the
                        // far zone, would then cost a failed search every
                        // `repath_ms` for as long as the clock allowed. Given up,
                        // each costs exactly one: the search is over, and the hunt
                        // starts its wait again.
                        if row
                            .last_plot
                            .is_some_and(|(cell, _)| row.goal == Some(cell))
                        {
                            row.last_plot = None;
                        }
                        if row.hunt.is_some_and(|cell| row.goal == Some(cell)) {
                            row.hunt = None;
                            row.unheld_since_ms = Some(t_ms);
                        }
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
        if routed.is_none() && !self.first_step_is_open(unit, here, heading, &row.avoid, counters) {
            plan.speed_cms = Some(0);
            return;
        }
        plan.heading_mdeg = Some(heading);
        plan.speed_cms = Some(braking_speed_cms(
            unit,
            goal,
            cell_cm,
            i64::from(memory.cfg.decide_ms),
        ));
    }

    /// **Whether the one cell on `heading` is a cell this unit would plan
    /// through** (16b cut Q.7).
    ///
    /// The neighbour is the octant of the bearing — the eight compass directions,
    /// each owning 45 degrees — because that is the cell
    /// [`crate::arena::motion`]'s step 4 will test first. Then four questions: it
    /// is on the window, its class is passable, the edge onto it is within this
    /// unit's slope limit **either way**, and on a diagonal its shoulders are not
    /// a cliff ([`shoulder_refuses`]). And one of Q.7's own: it is not a cell the
    /// unit has been stopped at ([`ReflexUnit::avoid`]).
    ///
    /// # It is the search's rule, not the crossing's — deliberately stricter
    ///
    /// **16b cut E.4 (16c `T-42`)** corrects what this paragraph used to claim.
    /// It said the four questions were asked "exactly as `motion`'s step 4 will
    /// ask" them, "in the crossing's own order", shoulder rule included. That is
    /// not true and cannot be: `crate::arena::motion::refusal` applies **no**
    /// shoulder rule at all — its own doc says a diagonal crossing tests the
    /// diagonal neighbour and nothing else — and the crossing knows nothing of a
    /// machine's remembered stops.
    ///
    /// What this really is: **the search's own edge rule
    /// ([`crate::arena::path`]'s, shoulders and all) plus the remembered stops**,
    /// which is strictly stricter than the crossing. That is the right way round
    /// and is the reason the function exists: a routeless machine is steering at
    /// a *direction* no search has checked, and the one thing it must never do is
    /// take a step its own search would refuse to plan — it would be stopped or
    /// turned aside, and then re-plan into the same step next tick. A cell motion
    /// would have let it through and the search would not costs it one held
    /// substep and a route a tick later; the other way round costs it the rest of
    /// the match.
    ///
    /// A unit with no slope limit at all is refused by no slope, exactly as
    /// `path` and `motion` refuse it by none.
    ///
    /// **16b cut U.1**: a refusal by the edge rule — the slope either way or the
    /// shoulders, after the class and the avoid list let the cell through — is
    /// counted in [`ReflexCounters::slope_refusals`].
    fn first_step_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        heading_mdeg: u32,
        avoid: &[((u16, u16), i64)],
        counters: &mut ReflexCounters,
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
        let open = edge_is_open(terrain, unit, here_cell, to_cell, here, (nx, ny));
        if !open {
            counters.slope_refusals = counters.slope_refusals.saturating_add(1);
        }
        open
    }

    /// **Whether every cell on the unit's heading within `reach_cm` is one it
    /// would plan through** (16b cut M.2, for [`Arena::hold_routeless`]).
    ///
    /// [`Arena::first_step_is_open`]'s four questions and Q.7's remembered stops,
    /// asked of each cell the unit's own line of travel enters: the pose is
    /// walked along its actual `pose.heading_mdeg` in steps of half a cell, with
    /// [`crate::arena::motion`]'s own fixed-point sine and cosine (no float: doc
    /// 16 §3.1's D-16-4), and every change of cell is one edge put to the
    /// search's rule. Half a cell is fine enough that the walk enters every cell
    /// the line does except where it clips a corner, and the corner it clips is
    /// the one the diagonal edge it then tests shoulders onto. A step off the
    /// window is a refusal, as the crossing's is.
    ///
    /// **The first cell the walk enters is not asked here**: the one cell in
    /// front is `first_step_is_open`'s question, asked exactly as Q.7 asked it
    /// (the octant the crossing tests first), and the walk only reaches the cells
    /// beyond it that the one-cell rule never looked at. This is measured, not
    /// tidiness: asking the walk's own first cell holds a unit that a stop has
    /// just clamped at a boundary — any reach at all, from there, is into the
    /// cell it was refused — so it can never turn away under power, re-plans from
    /// the same centimetre every `repath_ms` and is refused the same cell again;
    /// on the harness field that took one line tank to eight `blocked` lines at
    /// one cell and the field from 341 to 439. A `reach_cm` of nought walks
    /// nothing and answers `true`.
    fn way_ahead_is_open(
        &self,
        unit: &UnitState,
        here: (u16, u16),
        avoid: &[((u16, u16), i64)],
        reach_cm: i64,
    ) -> bool {
        use crate::arena::motion::{SIN_ONE, cos_fx, sin_fx};
        let terrain = &self.definition.terrain;
        let stride = (i64::from(terrain.cell_cm()) / 2).max(1);
        let east = sin_fx(unit.pose.heading_mdeg);
        let north = cos_fx(unit.pose.heading_mdeg);
        let mut from = here;
        let mut walked: i64 = 0;
        while walked < reach_cm {
            walked = walked.saturating_add(stride).min(reach_cm);
            let x = unit
                .pose
                .x_cm
                .saturating_add(walked.saturating_mul(east) / SIN_ONE);
            let y = unit
                .pose
                .y_cm
                .saturating_add(walked.saturating_mul(north) / SIN_ONE);
            let Some(to) = terrain.cell_at(x, y) else {
                return false;
            };
            if to == from {
                continue;
            }
            if from == here {
                from = to;
                continue;
            }
            let (Some(from_cell), Some(to_cell)) =
                (terrain.get(from.0, from.1), terrain.get(to.0, to.1))
            else {
                return false;
            };
            if !to_cell.passable()
                || avoid.iter().any(|(cell, _)| *cell == to)
                || !edge_is_open(terrain, unit, from_cell, to_cell, from, to)
            {
                return false;
            }
            from = to;
        }
        true
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
    ///
    /// **16b cut M.2** (16c `T-02`): that timestamp is `t_ms`, handed to the
    /// three resolvers. They read `now_ms` until this cut, and the tick runs
    /// after the substep has advanced `now_ms` to its end, so a machine's
    /// `fired` line carried the substep boundary and not the tick.
    fn order(&mut self, id: ObjId, plan: &ReflexPlan, t_ms: i64, counters: &mut ReflexCounters) {
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
            // **16b cut M.1** (`T-82`): the machine's own order, which the
            // per-seat rate never counts — see `IntentSource`.
            if apply_intent(self, &intent, IntentSource::Machine) != IntentOutcome::Queued {
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
                    let _ = resolve_startup(self, id, t_ms);
                }
                IntentKind::Lock { contact } => {
                    let _ = self.resolve_lock(id, *contact, t_ms);
                }
                IntentKind::Fire { weapon } => {
                    if self.resolve_fire(id, weapon, t_ms).is_ok() {
                        counters.shots = counters.shots.saturating_add(1);
                    }
                }
                IntentKind::Heading { .. } | IntentKind::Speed { .. } | IntentKind::Order(_) => {}
            }
        }
    }
}

/// **The speed a machine orders while it is closing on its goal** (16b cut E.3;
/// 16c `B-42`), so that it arrives there instead of driving through it.
///
/// Before this cut [`Arena::follow`] ordered `max_speed_cms` flat until the unit
/// was inside the one-cell arrive window and then ordered zero. A stock tank
/// needs 56 m to stop from 15 m/s at 2 m/s², and the controller decides once a
/// second, so the machines coasted through their goals: through the firing stop
/// 16b cut E.1 had just priced, and — for a withdrawing tank — past its own entry
/// zone and out the far side, restarting its rally clock.
///
/// The profile is the classical one, in whole centimetres and `u64` throughout
/// ([`u64::isqrt`]; doc 16 §3.1's D-16-4 forbids a float in arena state and this
/// is arena arithmetic):
///
/// ```text
/// d      = max(0, range(pose, goal cell centre) − ARRIVE_CELLS × cell_cm)
/// d_eff  = max(0, d − |speed_cms| × decide_ms / 1000)
/// v_stop = isqrt(2 × brake_cms2 × d_eff)
/// order  = min(max_speed_cms, v_stop)
/// ```
///
/// `d` is measured to the **goal** and not to the route's next cell, because the
/// route is consumed cell by cell and its end *is* the goal: braking for the next
/// cell would crawl the whole way. `d_eff` is the one-tick lookahead — where the
/// unit will be when the controller next gets to think — without which a profile
/// sampled at 1 Hz orders a speed the tank has already outrun by the time it
/// obeys it. A unit with no braking rate at all orders `max_speed_cms`, which is
/// the pre-cut behaviour and the only honest answer for a platform that cannot
/// slow down.
///
/// The range is planar: the goal cell's own elevation is not read, so a goal up a
/// hill is not braked for a few metres early. That is the same simplification
/// [`Arena::stand_off_cell`] makes, and the arrive window is a cell wide.
fn braking_speed_cms(unit: &UnitState, goal: (u16, u16), cell_cm: i64, decide_ms: i64) -> i32 {
    let ceiling = u64::from(unit.mobility.max_speed_cms);
    let brake = u64::from(unit.mobility.brake_cms2);
    let capped = i32::try_from(ceiling).unwrap_or(i32::MAX);
    if brake == 0 {
        return capped;
    }
    let side = cell_cm.max(1);
    let to_x = i64::from(goal.0)
        .saturating_mul(side)
        .saturating_add(side / 2);
    let to_y = i64::from(goal.1)
        .saturating_mul(side)
        .saturating_add(side / 2);
    let range = range_between(
        unit.pose.x_cm,
        unit.pose.y_cm,
        unit.pose.z_cm,
        to_x,
        to_y,
        unit.pose.z_cm,
    );
    let window = u64::from(ARRIVE_CELLS).saturating_mul(u64::try_from(side).unwrap_or(1));
    let distance = range.saturating_sub(window);
    let ahead = u64::from(unit.speed_cms.unsigned_abs())
        .saturating_mul(u64::try_from(decide_ms.max(0)).unwrap_or(0))
        / 1_000;
    let effective = distance.saturating_sub(ahead);
    let stopping = brake.saturating_mul(2).saturating_mul(effective).isqrt();
    i32::try_from(ceiling.min(stopping)).unwrap_or(i32::MAX)
}

/// **How far ahead a routeless machine must be able to go** (16b cut M.2, for
/// [`Arena::hold_routeless`]), centimetres: the distance it needs to stop from
/// the fastest it can be going at the end of the next substep, plus what it
/// covers in that substep before the pre-pass asks again.
///
/// ```text
/// v      = max(0, speed_cms)
/// v_next = min(v + accel_cms2 × substep_ms / 1000, max(v, desired_speed_cms))
/// reach  = v_next² / (2 × brake_cms2) + v_next × substep_ms / 1000
/// ```
///
/// `v_next` bounds the speed from above: a unit ordered faster than it is going
/// gains at most one substep of acceleration, and one ordered slower is already
/// braking. The travel term is at the same bound, so it also covers what the
/// integration's per-substep speed update adds over the continuous stopping
/// distance. Integer arithmetic throughout (doc 16 §3.1's D-16-4). A platform
/// with no braking rate answers nought — it cannot stop in any distance, so the
/// look-ahead has nothing to buy it, which is [`braking_speed_cms`]'s rule — and
/// so does one moving backwards, whose way ahead is not on its heading.
fn stopping_reach_cm(unit: &UnitState, substep_ms: i64) -> i64 {
    let brake = i64::from(unit.mobility.brake_cms2);
    if brake == 0 {
        return 0;
    }
    let speed = i64::from(unit.speed_cms.max(0));
    let gained = speed.saturating_add(
        i64::from(unit.mobility.accel_cms2).saturating_mul(substep_ms.max(0)) / 1_000,
    );
    let next = gained.min(speed.max(i64::from(unit.desired_speed_cms)));
    next.saturating_mul(next) / brake.saturating_mul(2)
        + next.saturating_mul(substep_ms.max(0)) / 1_000
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
/// **It is the engine's own line, and exactly it** (16b cut E.2, 16c `T-55`).
///
/// The engine refuses **at** the ceiling and accepts and clamps below it:
/// [`crate::arena::state::UnitState::overheated`] is `now >= max`, and
/// `Arena::raise_heat` writes `min(now + per_shot, max)`. So does the machine —
/// this is [`crate::arena::state::UnitState::overheated`] and nothing else.
///
/// Until this cut it declined a shot that would land the unit **above** the
/// ceiling, one whole shot short of the engine's line, so a pilot could fire a
/// round a machine in the same tank could not: the two halves of one platform
/// disagreeing about whether the gun works, which is the very sentence this
/// paragraph used to make the other argument with. A hot shot below the ceiling
/// is a shot the resolver accepts and clamps, so the machine takes it and cooks
/// itself to the ceiling exactly as a crew would; the **next** one is refused,
/// by both halves, at the same number.
///
/// A unit already at the ceiling answers `true` for every weapon **that makes
/// heat** — **16b cut D.2** (`B-16`) took the zero out of that set, in both
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
    if unit.heat_now().is_none() {
        return false;
    }
    // 16b cut D.2 (`B-16`): a cold gun is never held back, at the ceiling or
    // anywhere else.
    if heat.per_shot(weapon) == 0 {
        return false;
    }
    unit.overheated()
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
    ///
    /// **16b cut E.2** (16c `T-55`) made the same sentence true one row higher:
    /// the copy is now the engine's predicate exactly, so a **hot** gun one shot
    /// short of the ceiling is one both halves fire.
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
        // One shot short of the ceiling. **16b cut E.2 (16c `T-55`) reversed
        // this row**: the engine accepts a shot that would carry a unit past the
        // maximum and clamps it there (`Arena::raise_heat`), and it refuses only
        // AT the ceiling, so the machine takes this one too. Before the cut the
        // hot gun read `true` here — a round a pilot in the same seat could fire
        // and a machine could not.
        set_heat(&mut arena, ObjId(4), 50);
        assert_eq!(asked(&arena), (false, false));
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

    /// **The stock tank's three rows of [`Ranges`]'s table** (16b cut E.1), and
    /// the property they exist for: `release_cm > engage_cm` under **every**
    /// order band, so a target drifting about the engagement distance cannot flip
    /// the machine between stopping to shoot and driving to a stand-off cell.
    ///
    /// The shipped numbers, written here rather than read off the pack: a 60-cell
    /// sight of 1,000 cm cells, a cannon whose bands are 1.5 km and 3 km, and
    /// doctrine's `engage_band_permille` of 700.
    /// `crates/tacsim/tests/content.rs` is what keeps them the pack's.
    #[test]
    fn e1_the_stock_tanks_three_rows_of_the_range_table() {
        use crate::arena::perception::{SensorProfile, SignatureFactor};
        use crate::arena::state::WeaponSummary;
        use crate::arena::weapons::{DEFAULT_WEAPON, WeaponBand};
        use crate::deliberation::order::{Objective, Order};
        use crate::deliberation::squad::SquadId;

        let mut fixture = tank(ObjId(4), Team::Red, pose(1_500, 1_500));
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
        let arena = arena_of(vec![fixture]);
        let Some(state) = arena.unit(ObjId(4)) else {
            panic!("the fixture tank")
        };
        assert_eq!(state.sight_cm(1_000), 60_000, "600 m of scope");
        assert_eq!(state.fight_inner_cm(1_000), 60_000);
        assert_eq!(state.fight_limit_cm(1_000), 60_000);

        let cfg = ReflexConfig::default();
        let ordered = |band: Option<crate::deliberation::order::RangeBand>| Order {
            squad: SquadId {
                team: Team::Red,
                index: 1,
            },
            objective: Objective::Engage { target_hint: None },
            target: None,
            horizon_ms: 30_000,
            constraints: crate::deliberation::order::Constraints {
                hold_fire: false,
                max_range_band: band,
            },
            ttl_ms: 30_000,
            issued_ms: 0,
        };
        for (band, engage_cm, release_cm, stand_cm) in [
            (None, 42_000_u64, 60_000_u64, 20_000_u64),
            (Some(RangeBand::Far), 42_000, 60_000, 20_000),
            (Some(RangeBand::Effective), 28_000, 40_000, 20_000),
            (Some(RangeBand::Close), 14_000, 20_000, 20_000),
        ] {
            let order = ordered(band);
            let have = ranges(state, 1_000, &cfg, Some(&order));
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
            ranges(state, 1_000, &cfg, None),
            ranges(state, 1_000, &cfg, Some(&ordered(Some(RangeBand::Far))))
        );

        // An operator who writes one thousand has asked for one distance and
        // gets it — which is the key's own doc, pinned.
        let flat = ReflexConfig {
            engage_band_permille: 1_000,
            ..cfg.clone()
        };
        let have = ranges(state, 1_000, &flat, None);
        assert_eq!((have.engage_cm, have.release_cm), (60_000, 60_000));

        // **A short first band**: the `close` band is a third of the *last* edge
        // and `release_cm` is the *first* one, so a gun whose inner band is under
        // a third of its outer reach would leave an en-route stand wider than the
        // firing ceiling — a hurt machine stopping in front of something it may
        // not shoot at. Nothing the game ships is shaped that way; the cap is
        // here so that nothing authored later can be.
        let mut stubby = arena;
        if let Some(state) = stubby.unit_mut(ObjId(4))
            && let Some(weapon) = state.weapons.first_mut()
            && let Some(band) = weapon.bands.first_mut()
        {
            band.max_range_cm = 10_000;
        }
        let Some(state) = stubby.unit(ObjId(4)) else {
            panic!("the fixture tank")
        };
        let have = ranges(state, 1_000, &cfg, None);
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
        let Some(state) = dry.unit(ObjId(4)) else {
            panic!("the fixture tank")
        };
        assert_eq!(ranges(state, 1_000, &cfg, None), Ranges::default());
    }

    /// **[`stand_off_want`] as a table** (16b cut E.1; 16c `B-05`).
    ///
    /// Three claims over every row, and each of them is a way the first draft of
    /// this arithmetic went wrong: the answer is never above `range_cm` (a want
    /// past the range projects the goal *behind* the machine), it is never nought
    /// unless the range is (a nought want puts the goal on the plot's own cell),
    /// and nothing underflows a `u64` for a band shorter than the margin.
    #[test]
    fn e1_the_stand_off_want_is_never_past_the_range_and_never_underflows() {
        let cell = 1_000_u64;
        for (engage_cm, range_cm, want) in [
            // Nothing loaded: the floor of one cell, held to the range.
            (0_u64, 0_u64, 0_u64),
            (0, 500, 500),
            (0, 42_000, 1_000),
            // A band of exactly one cell, and of exactly the margin: the
            // subtraction would underflow, and the floor answers instead.
            (1_000, 42_000, 1_000),
            (3_000, 42_000, 1_000),
            (3_001, 42_000, 1_000),
            // The stock tank's own row: 420 m less three cells is 390 m.
            (42_000, 42_000, 39_000),
            (42_000, 60_000, 39_000),
            // And a range inside the want: the want comes down to it.
            (42_000, 20_000, 20_000),
            (42_000, 900, 900),
            // An engagement band bigger than anything on the window.
            (u64::MAX, 42_000, 42_000),
        ] {
            let have = stand_off_want(engage_cm, cell, range_cm);
            assert_eq!(have, want, "engage {engage_cm}, range {range_cm}");
            assert!(
                have <= range_cm,
                "engage {engage_cm}, range {range_cm}: {have} is past the range, so the goal \
                 would be behind the machine"
            );
            assert!(
                have > 0 || range_cm == 0,
                "engage {engage_cm}, range {range_cm}: a nought want is the plot's own cell"
            );
        }
        // And a window with a nonsense cell size does not panic either.
        assert_eq!(stand_off_want(42_000, u64::MAX, 42_000), 42_000);
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
        let mut memory = ReflexMemory::new(&cfg);
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
        let mut memory = ReflexMemory::new(&cfg);
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

        // **16b cut E.3 (16c `B-43`): the list evicts the LEAST RECENTLY STOPPED
        // AT and not the entry at index zero.** The two orders differ exactly
        // because a repeat refreshes a stamp in place: `(1, 1)` was the oldest
        // insert and is now the newest stop, so a list that evicted by insertion
        // order would drop the one cell the unit keeps being stopped at — the
        // wreck — and keep seven it drove past once. One more distinct cell
        // overflows the list: `(1, 1)` survives and `(2, 1)`, whose stamp is the
        // least, goes.
        arena.note_blocked(ObjId(4), (9, 1), BlockedCause::Occupied, 9_500);
        let row = arena.reflex.unit(ObjId(4)).cloned().unwrap_or_default();
        assert_eq!(row.avoid.len(), REFLEX_AVOID_MAX);
        let cells: Vec<(u16, u16)> = row.avoid.iter().map(|(cell, _)| *cell).collect();
        assert!(
            cells.contains(&(1, 1)),
            "the refreshed cell was evicted by its insertion order: {cells:?}"
        );
        assert!(
            !cells.contains(&(2, 1)),
            "the least recently stopped at survived: {cells:?}"
        );
        assert!(cells.contains(&(9, 1)), "the new stop is in: {cells:?}");

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

    /// **The pre-pass orders the one machine it is for, counts the order, and
    /// leaves everybody else alone** (16b cut E.3; 16c `B-44`).
    ///
    /// [`Arena::hold_routeless`] is five guards and one question, and every guard
    /// is load-bearing: the route test is what keeps a follower in a column out of
    /// it (16c `B-46` on the one ground the match uses), the controller test is
    /// what keeps it off a seated pilot's tank, and the power and speed tests are
    /// what keep it from ordering a stop onto a tank that is already standing
    /// still. None of them is reachable from an end-to-end fixture, because
    /// `Arena::follow`'s own tick-time check answers the same question a tick
    /// later and hides the difference; so each is asked here directly, of the
    /// function, with the count it is supposed to leave behind.
    ///
    /// The window is `test_support`'s eight by eight of passable ground, and the
    /// closed step is the machine's **own remembered stop** rather than a painted
    /// cliff: `first_step_is_open`'s `avoid` clause is the cheapest way to shut one
    /// cell without moving the ground under the other cases.
    #[test]
    fn e3_the_pre_pass_holds_a_routeless_machine_and_skips_everyone_else() {
        use crate::arena::state::Power;

        /// One case: build the field, let `edit` bend one thing about it, run the
        /// pre-pass once, and report what the unit was left ordering and what the
        /// pass counted.
        fn asked(edit: impl FnOnce(&mut Arena)) -> (i32, u32) {
            // Cell (3, 3) of a 1,000 cm window, facing north, which
            // `octant_step(0)` makes the step onto (3, 4).
            let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(3_500, 3_500))]);
            if let Some(state) = arena.unit_mut(ObjId(4)) {
                state.controller = Controller::Reflex;
                state.power = Power::Online;
                state.desired_speed_cms = 800;
            }
            arena.reflex.adopt(ObjId(4), ReflexState::Patrol);
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                row.goal = Some((3, 7));
                row.route.clear();
                row.avoid = vec![((3, 4), 0)];
            }
            edit(&mut arena);
            let mut counters = ReflexCounters::default();
            arena.hold_routeless(&mut counters);
            (
                arena
                    .unit(ObjId(4))
                    .map_or(i32::MIN, |state| state.desired_speed_cms),
                counters.intents,
            )
        }

        // The case the pass exists for: a machine with a goal, no route, way on,
        // and the cell in front of it one it has just been stopped at.
        assert_eq!(
            asked(|_| {}),
            (0, 1),
            "a routeless machine pointed at a closed cell was not ordered to stand, or the order \
             was not counted"
        );
        // A machine whose route is not empty is a machine the search has already
        // checked the ground for — every follower in a column at the ford.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                    row.route = vec![(3, 4), (3, 5)];
                }
            }),
            (800, 0),
            "the pass took a machine that had a route"
        );
        // A machine with nowhere to be is not being driven anywhere.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                    row.goal = None;
                }
            }),
            (800, 0),
            "the pass took a machine with no goal"
        );
        // **A seated pilot's tank is theirs.** The controller does nothing a pilot
        // cannot, and it does nothing to a unit it is not flying.
        assert_eq!(
            asked(|arena| {
                if let Some(state) = arena.unit_mut(ObjId(4)) {
                    state.controller = Controller::Pilot;
                }
            }),
            (800, 0),
            "the pass ordered a seated pilot's tank to stop"
        );
        // A cold hull is not going anywhere and `apply_intent` would refuse the
        // order anyway; the pass does not pay for the map lookup to find out.
        assert_eq!(
            asked(|arena| {
                if let Some(state) = arena.unit_mut(ObjId(4)) {
                    state.power = Power::Starting;
                }
            }),
            (800, 0),
            "the pass took a hull whose power was not up"
        );
        // Already standing: nothing to order and nothing to count.
        assert_eq!(
            asked(|arena| {
                if let Some(state) = arena.unit_mut(ObjId(4)) {
                    state.desired_speed_cms = 0;
                }
            }),
            (0, 0),
            "the pass re-ordered a stop onto a machine that was already standing"
        );
        // And the question itself: with the cell in front open, the machine keeps
        // its way.
        assert_eq!(
            asked(|arena| {
                if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                    row.avoid.clear();
                }
            }),
            (800, 0),
            "the pass stopped a machine whose next cell was open"
        );
    }

    /// **16b cut U.1**: the follower's one-cell test counts a refusal by the
    /// **edge rule** in [`ReflexCounters::slope_refusals`] — here an 8 m bluff
    /// in front of a unit with a 45 per cent limit, an 80 per cent edge — and a
    /// refusal by the machine's own remembered stop counts none: that cell is
    /// shut by the avoid list, not by the slope.
    #[test]
    fn u1_the_one_cell_test_counts_a_slope_refusal_and_not_a_remembered_stop() {
        use crate::arena::state::{ArenaTerrain, Power};
        use crate::arena::test_support::ground;

        /// Cell (3, 3) facing north onto (3, 4), as in the E.3 fixture; `bluff`
        /// raises (3, 4) by 800 cm and `remembered` puts it on the avoid list.
        fn asked(bluff: bool, remembered: bool) -> (i32, u32) {
            let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(3_500, 3_500))]);
            if bluff {
                let mut cells = vec![ground(); 8 * 8];
                if let Some(slot) = cells.get_mut(4 * 8 + 3) {
                    slot.elev_cm = 800;
                }
                arena.definition.terrain =
                    ArenaTerrain::new(8, 8, 1_000, cells).expect("the bluff window");
            }
            if let Some(state) = arena.unit_mut(ObjId(4)) {
                state.controller = Controller::Reflex;
                state.power = Power::Online;
                state.desired_speed_cms = 800;
                state.slope_limit_pct = Some(45);
            }
            arena.reflex.adopt(ObjId(4), ReflexState::Patrol);
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                row.goal = Some((3, 7));
                row.route.clear();
                row.avoid = if remembered {
                    vec![((3, 4), 0)]
                } else {
                    Vec::new()
                };
            }
            let mut counters = ReflexCounters::default();
            arena.hold_routeless(&mut counters);
            (
                arena
                    .unit(ObjId(4))
                    .map_or(i32::MIN, |state| state.desired_speed_cms),
                counters.slope_refusals,
            )
        }

        assert_eq!(
            asked(true, false),
            (0, 1),
            "the bluff in front did not hold the machine, or was not counted as a slope refusal"
        );
        assert_eq!(
            asked(false, true),
            (0, 0),
            "a remembered stop was counted as a slope refusal"
        );
        assert_eq!(
            asked(false, false),
            (800, 0),
            "level ground refused an edge"
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
            // **16b cut E.4**: the last plot, then its cell alone and its
            // stamp alone against it — a machine that saw the same cell a
            // second later is a different machine, because `search_ms` is
            // measured off that stamp, and so is one that saw a different
            // cell at the same moment — then the hunt's clock, and the hunt's
            // cell twice. Each probe is checked against every earlier one, so
            // 6 against 5 is the cell alone, 7 against 5 the stamp alone, and
            // 10 against 9 the hunt's cell alone: a fold that wrote only the
            // presence byte and the stamp would pass the first probe of each
            // and be caught by the second. `last_plot`'s first cell is
            // deliberately the same `(3, 4)` the hunt's first is, so a fold
            // that mixed the two would be caught here and not only by a
            // cadence.
            5,  // last_plot
            6,  // last_plot's cell alone
            7,  // last_plot's stamp alone
            8,  // unheld_since_ms
            9,  // hunt
            10, // hunt's cell alone
        ] {
            let mut arena = build();
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                match edit {
                    0 => row.avoid.push(((3, 4), 1_000)),
                    1 => row.avoid.push(((3, 4), 2_000)),
                    2 => row.stand_until_ms = Some(1_500),
                    3 => row.quiet_since_ms = Some(1_500),
                    4 => row.rallied = true,
                    5 => row.last_plot = Some(((3, 4), 1_000)),
                    6 => row.last_plot = Some(((5, 6), 1_000)),
                    7 => row.last_plot = Some(((3, 4), 2_000)),
                    8 => row.unheld_since_ms = Some(1_500),
                    9 => row.hunt = Some((3, 4)),
                    _ => row.hunt = Some((5, 6)),
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

    /// **`adopt` hands the new state a clean endgame memory** (16b cut E.4): a
    /// seat released into `Withdraw` must not inherit its pilot-era plot as a
    /// place to drive to, nor a crossing, nor a clock that has been running
    /// since before the machine had the tank.
    #[test]
    fn adopt_forgets_the_search_and_the_hunt() {
        let mut memory = ReflexMemory::default();
        memory.adopt(ObjId(4), ReflexState::Patrol);
        if let Some(row) = memory.units.get_mut(&ObjId(4)) {
            row.last_plot = Some(((3, 4), 1_000));
            row.unheld_since_ms = Some(1_500);
            row.hunt = Some((5, 6));
        }
        // The same state again is idempotent: a second `Control` of the same
        // kind does not restart a machine halfway through a patrol.
        memory.adopt(ObjId(4), ReflexState::Patrol);
        assert_eq!(
            memory
                .unit(ObjId(4))
                .and_then(|row| row.last_plot.map(|(cell, _)| cell)),
            Some((3, 4))
        );
        memory.adopt(ObjId(4), ReflexState::Withdraw);
        let row = memory.unit(ObjId(4)).expect("the row");
        assert_eq!(row.last_plot, None, "the plot survived the adoption");
        assert_eq!(row.unheld_since_ms, None, "the clock survived the adoption");
        assert_eq!(row.hunt, None, "the crossing survived the adoption");
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
