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
//! are bounded five ways: [`crate::arena::path::path`] expands at most
//! `reflex.path_max_cells` nodes (16b §9's raised `[atlas].path_max_cells`); one
//! unit searches at most once per `reflex.repath_ms`; and the whole field searches
//! at most `reflex.max_path_queries` times per `reflex.decide_ms` **window of
//! arena time**, a unit refused by that budget keeping the path it has. The window
//! is arena time and not a pulse for D-16-2's reason once more;
//! `[tacsim.harness].max_path_queries_per_pulse` stays what its own block says it
//! is, a threshold the harness asserts against [`ReflexCounters::path_queries`],
//! and **16b cut AI.1**'s `[tacsim.harness].max_path_cells_per_pulse` is the
//! same kind of threshold on [`ReflexCounters::path_cells`], which counts every
//! search's expansions, found or failed — never a throttle the arena applies.
//!
//! The fourth and fifth bounds are **16b cut AI.1**'s (16c `B-84`), and they
//! bound how often a machine *has to* search rather than how much a search may
//! cost. The fourth is the **lookahead**: every scan the follower makes of its
//! route — what it has passed, where it steers, whether it has wandered — reads
//! at most [`STEER_LOOKAHEAD_CELLS`] cells of it, so a tick costs at most eight
//! open-line walks however long the route — sixteen on a tick the wander rule
//! is asked, eight for it and eight for the aim. The fifth is the **wander rule**: a
//! machine whose route's next cell is no longer a neighbour keeps the route while
//! the nearest of those cells is within its turning circle's diameter and a cell
//! of it has an open line — a machine inside its circle is turning onto its
//! route, not lost — so the full-width search Q.7's wandered-off branch bought
//! every tick of an orbit (7,000 to 16,000 cells for a `Withdraw` making for its
//! entry zone) is bought only by a machine that really is off its route.
//!
//! # Deciding, and driving (17b cut M1a.1; doc 17 §4, D-UM-3)
//!
//! Every bound above is now [`crate::arena::motor`]'s. This module decides —
//! the state machine, the utility score, the goal precedence, the clocks — and
//! hands the hull a goal; the motor drives it there: the route, the search and
//! its budget, the follower's seven steps, the open-line walks, the remembered
//! stops and the per-substep pre-pass, one [`crate::arena::motor::MotorRow`] per
//! hull on [`Arena::motors`]. The nine driving fields left [`ReflexUnit`] for
//! that row and the hash folds them in the order they were folded here, so the
//! split moved no byte of any match. The constants and the stop memory moved
//! with them and are re-exported below under their old paths.

use std::collections::BTreeMap;

use mush_atlas::Cover;
use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::arena::intents::{
    Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, apply_intent, resolve_emcon,
    resolve_startup,
};
use crate::arena::ledger::EventKind;
use crate::arena::motor::{Fallback, MotorPlan, MotorRow, MotorTable};
use crate::arena::resolution::facet_band_seen;
use crate::arena::state::{
    Arena, Channel, Classification, ContactId, Controller, DueKind, Power, SeenPose, UnitState,
};
use crate::config::ReflexConfig;
use crate::deliberation::order::{Objective, Order, RangeBand, StandingOrder};

/// The motor's constants and its stop memory, under the paths they had while
/// they were this module's (17b cut M1a.1): `crate::arena`'s own re-exports and
/// every test's `use` lines resolve unchanged.
pub use crate::arena::motor::{
    ARRIVE_CELLS, HUNT_FAN_ROWS, MAX_ENTRY_SCAN, MDEG_PER_RADIAN, REFLEX_AVOID_MAX,
    REFLEX_AVOID_TTL_MS, Refusal, STAND_OFF_MARGIN_CELLS, STEER_LOOKAHEAD_CELLS,
};

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
/// the score — this module's, and since 17b cut M1a.6 the stock mind's
/// `best_hostile`, which is the same score over the view (hence `pub(crate)`).
pub(crate) const EXPOSURE_FRONT: u32 = 1_000;
/// A flank.
pub(crate) const EXPOSURE_FLANK: u32 = 600;
/// The rear.
pub(crate) const EXPOSURE_REAR: u32 = 200;

/// **Half the arc a screen covers**, millidegrees either side of its facing (16b
/// cut Z.2; 16c N6): `Objective::Screen` locks and fires on a classified hostile
/// whose plot bears inside this of `facing_mdeg`, and on nothing else.
///
/// Sixty degrees, so a 120° sector: wide enough that a tank crossing the front
/// is met, narrow enough that a screen has a flank — a screen that saw all
/// round would be a `Hold` that shoots, and the order would say nothing about
/// where the squad is looking.
pub const SCREEN_HALF_ARC_MDEG: u32 = 60_000;

/// **How far a supporter may stray from its charge**, cells (16b cut Z.2; 16c
/// N6): past this, measured to the supporter's own plot of the charge, the leash
/// wins over the chase and the supporter drives back toward its station.
///
/// Fifteen, which is 150 m on the shipped ground: inside the `close` band of the
/// stock cannon, so a supporter fights the same target its charge fights rather
/// than one of its own a ridge away.
pub const SUPPORT_LEASH_CELLS: u16 = 15;

/// **The station a supporter keeps on its charge**, cells (16b cut Z.2; 16c N6):
/// near, not on top. The station cell — projected back from the plot of the
/// charge toward the supporter — is the goal until the unit stands inside this
/// of the plot, and then it holds.
pub const SUPPORT_STATION_CELLS: u16 = 5;

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

/// **Where one AI unit is in doc 16 §6's machine.**
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// 4. **the waypoint walk** — `Arena::patrol_step` (the motor's since 17b
    ///    cut M1a.1), the arena pack's graph, which is what `Patrol` was and
    ///    still is when the three above have nothing to say.
    ///
    /// **16b cut Z.2**: a machine under `Objective::Screen` stays here — parked,
    /// faced onto the order's bearing — and fires from its tick's *stand*
    /// without ever entering `Approach` or `Engage`, so a screening tick reads
    /// `patrol` in the state distribution. So does a supporter holding its
    /// station, or standing where it is because it cannot see its charge.
    ///
    /// **16b cut O.1 (16c N11; plan items 2, 16, 32, 33): in a Hold match the
    /// key spot is doctrine's fallback goal.** The list is then 1. the movement
    /// order, 2. the search, 3. **the Hold spot** (`Arena::hold_goal`) in place
    /// of BOTH the hunt and the waypoint walk, and 4. the waypoint walk only
    /// while `hold_goal` declines — a spot no route reaches, given up for
    /// `max(hunt_after_ms, repath_ms)` ([`MotorRow::hold_retry_ms`]). A
    /// machine arrives anywhere inside `radius − 1` of the spot and parks there
    /// (presence is all a hold needs, and one cell cannot seat a side); it is
    /// **not** a standing `Objective::Advance` written by the arena, because a
    /// live `Advance` suppresses the hostile edges for its whole TTL and arrival
    /// does not end it. A hostile sighted from the spot is fought **from inside
    /// the square** (the `Approach` arm's inside rule), and an `Advance` a
    /// commander sends into the square hands over to this goal on arrival
    /// (beside AA.1's arm). Both sides' machines take it. What is NOT changed,
    /// recorded for the owner in Q-O1: an attacking machine stops at
    /// `engage_cm` and never contests the square.
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
/// which state it is in, which hostile it settled on, the order it is under and
/// the clocks of its endgame.
///
/// **17b cut M1a.1** (doc 17 §4, D-UM-3): where it is going, the route it is
/// following and how far along it, when and from where it last searched, the
/// cells it was stopped at and the stop it remembers, the waypoint walk and the
/// Hold retry clock — the nine driving fields — left this row for the hull's
/// [`MotorRow`] on [`Arena::motors`], each with its doc. What is left here is
/// the decision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflexUnit {
    /// Where it is in the machine.
    pub state: ReflexState,
    /// The hostile it settled on, and the contact it holds it under.
    pub target: Option<(ContactId, ObjId)>,
    /// A shot of this unit's was refused the line (W's `blocked` or `cover` miss):
    /// the `no line` edge, cleared by the tick that re-plans on it.
    pub no_line: bool,
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

impl ReflexUnit {
    /// **What doctrine restores when an order stops steering** (16b cut N.2's
    /// expiry, factored by 16b cut Z.2 so a pilot's `RELEASE` is the same
    /// restoration and not a second one): the order, the goal, the route and the
    /// target are dropped, so a patrol resumes the waypoint graph and an
    /// engagement is re-chosen by the utility score.
    ///
    /// A `Withdraw` the **order** put the machine in is undone as well, but only
    /// when `health_ok` — the unit's own health does not ask for one — otherwise
    /// the machine would answer the end of an order by driving a wreck back into
    /// the fight. So a released `Withdraw` does not keep driving to its entry
    /// zone, and a hurt one keeps going.
    ///
    /// **17b cut M1a.1**: the goal, the route and the remembered stop are the
    /// hull's [`MotorRow`] now, so this drops the decision's half — the order
    /// and the target, and the `Withdraw` — and each caller drops the motor's
    /// half beside it ([`MotorTable::forget_goal`], or the row the decision
    /// holds), which is the same restoration in two rows.
    pub fn drop_to_doctrine(&mut self, health_ok: bool) {
        self.order = None;
        self.target = None;
        if self.state == ReflexState::Withdraw && health_ok {
            self.state = ReflexState::Patrol;
        }
    }
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
    /// Nodes those searches expanded — **found or failed** since 16b cut AI.1
    /// (16c `B-84`): a `Budget` failure is `path_max_cells` expansions and a
    /// `NoRoute` the whole reachable region, the two most expensive searches
    /// there are, and until that cut they counted nothing here. What
    /// `harness.max_path_cells_per_pulse` bounds.
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
    /// **Orders a pilot handed back** (16b cut Z.2; 16c Q44), one per unit a
    /// `ReleaseOrder` dropped to doctrine — counted at apply, in pulse phase 2.
    pub orders_released: u32,
    /// Ticks that ended in each state, in [`ReflexState::ALL`] order.
    pub in_state: [u32; 6],
    /// **Edges refused by the slope rule** (16b cut U.1; 16c `T-35`): every edge
    /// a search refused as too steep either way or by the shoulder rule
    /// ([`crate::arena::path::PathScratch::slope_refusals`]), plus every time the
    /// follower's one-cell test (`Arena::first_step_is_open`) found the cell in
    /// front open to the class and the avoid list and refused it by the same
    /// edge rule. What the harness asserts non-zero to prove the cohort's
    /// locomotion met the relief; a counter, not state, so it moves no hash.
    ///
    /// **16b cut AI.1**: a corner refused because no shoulder is open is not a
    /// slope refusal and is counted in [`ReflexCounters::corner_refusals`]
    /// instead, so this figure keeps its meaning.
    pub slope_refusals: u32,
    /// **Searches refused by name** (16b cut AI.1; 16c `B-84`): ticks at which a
    /// due search was not spent because [`MotorRow::refused`] said it would
    /// return the route that had just been refused, and the machine held
    /// instead. **Expected nought on every lane**: the memory is unreachable
    /// through `Arena::step` at this cut's rules, and a count is a controller
    /// defect to investigate. A counter, never hashed.
    pub path_refused_unchanged: u32,
    /// **Diagonals refused as a corner closed on both shoulders** (16b cut
    /// AI.1; 16c `B-84`): every one a search refused
    /// ([`crate::arena::path::PathScratch::corner_refusals`]), plus every time
    /// the follower's one-cell test found the diagonal in front open to the
    /// class and the avoid list and refused it because neither of its shoulders
    /// is. A counter, never hashed.
    pub corner_refusals: u32,
    /// **Unit minds' thinks applied** (17b cut M1a.5; doc 17 §6): one per
    /// `MindThink` that called its runner. Not reported by the harness — no
    /// production unit is a mind — and never hashed.
    pub thinks: u32,
    /// **Thinks that answered more than `MIND_INTENTS_MAX` intents** (17b cut
    /// M1a.5; D-UM-9) and so landed nothing. Not reported; 17b cut M3 journals
    /// the fault. Never hashed.
    pub mind_overflows: u32,
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
        self.orders_released = self.orders_released.saturating_add(other.orders_released);
        self.slope_refusals = self.slope_refusals.saturating_add(other.slope_refusals);
        self.path_refused_unchanged = self
            .path_refused_unchanged
            .saturating_add(other.path_refused_unchanged);
        self.corner_refusals = self.corner_refusals.saturating_add(other.corner_refusals);
        self.thinks = self.thinks.saturating_add(other.thinks);
        self.mind_overflows = self.mind_overflows.saturating_add(other.mind_overflows);
        for (mine, theirs) in self.in_state.iter_mut().zip(other.in_state.iter()) {
            *mine = mine.saturating_add(*theirs);
        }
    }
}

// ---------------------------------------------------------------------------
// The memory
// ---------------------------------------------------------------------------

/// **Everything the controller holds for the life of one match**: the per-unit
/// machines and the frozen doctrine.
///
/// It lives on [`Arena`] rather than on `UnitState` because a unit under a pilot
/// carries no machine at all.
///
/// **17b cut M1a.1** (doc 17 §4, D-UM-3): the A\* scratch and the path budget's
/// window and spend moved to [`MotorTable`] ([`Arena::motors`]) with the
/// driving fields they serve; the budget is every steered hull's, not one
/// controller's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflexMemory {
    units: BTreeMap<ObjId, ReflexUnit>,
    cfg: ReflexConfig,
}

impl ReflexMemory {
    /// An empty memory with this match's doctrine frozen in, exactly as the two
    /// configuration blocks are frozen into [`Arena`].
    ///
    /// **16b cut M.4** (16c `T-15`): the window's cheapest cell, which 16b cut
    /// N.1 kept here for the A\* heuristic, moved to
    /// [`Arena::min_cost`](crate::arena::state::Arena::min_cost) (one per row
    /// since 16b cut AE.1), one copy for the search and the speed ceiling both.
    #[must_use]
    pub fn new(cfg: &ReflexConfig) -> ReflexMemory {
        ReflexMemory {
            units: BTreeMap::new(),
            cfg: cfg.clone(),
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
    ///
    /// **17b cut M1a.1**: answers whether it pointed the machine at a new state —
    /// the case in which the hull's [`MotorRow`] is reset too — and resets only
    /// the decision's fields itself. Called through [`Arena::adopt_reflex`], which
    /// does both, and never alone outside this module's own tests.
    pub fn adopt(&mut self, unit: ObjId, state: ReflexState) -> bool {
        let row = self.units.entry(unit).or_default();
        if row.state != state {
            row.state = state;
            // **16b cut Q.7**: a machine pointed at a new state starts with a
            // clean memory of where it was stopped and whether it has already
            // rallied. A seat released into `Withdraw` is the case that matters:
            // it becomes an AI teammate that can come back, not one that has
            // already used its one return. (Where it was stopped is the motor
            // row's since 17b cut M1a.1, reset by `Arena::adopt_reflex`.)
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
            return true;
        }
        false
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

    /// **Drop one unit's machine to doctrine** (16b cut Z.2; 16c Q44): the
    /// standing order and the target are forgotten, and a `Withdraw` becomes
    /// `Patrol` when `health_ok` — see [`ReflexUnit::drop_to_doctrine`], which is
    /// the one way of doing it; the expiry block and a pilot's `RELEASE` both
    /// come through it. The goal and the route are the hull's [`MotorRow`]'s
    /// since 17b cut M1a.1: the caller forgets them beside this
    /// ([`MotorTable::forget_goal`]).
    ///
    /// `health_ok` is the caller's `reflex_health_permille(unit) >=
    /// withdraw_permille`: the memory holds no arena and cannot read the hull. A
    /// unit with no machine is left alone.
    pub fn drop_to_doctrine(&mut self, unit: ObjId, health_ok: bool) {
        if let Some(row) = self.units.get_mut(&unit) {
            row.drop_to_doctrine(health_ok);
        }
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
pub(crate) struct ReflexPlan {
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
///
/// **Shared arithmetic, not a leaky reader** (17b cut M1a.6): `pub` because the
/// stock mind's [`crate::arena::stock_mind::view_ranges`] answers this struct
/// over the view, so T0 and T1 fight at the same three distances. It outlives
/// the reflex's decision half: when 17b cut M4 deletes that, `Ranges` and
/// [`stand_off_want`] move beside the stock mind. Only [`ranges`] — the reading
/// off the live [`UnitState`] — is T0's and leaves with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ranges {
    /// Where `Approach` stops and turns: the doctrine band of the allowed reach —
    /// for a machine that can close (16b cut AA.1).
    pub engage_cm: u64,
    /// Where `Engage` gives up and closes again — **and the ceiling
    /// `Arena::engage_orders` will fire out to**, so the two cannot disagree.
    pub release_cm: u64,
    /// What a `Withdraw` still on its way home will turn and fight inside: the
    /// `close` band, capped by the order **and by `release_cm`**.
    ///
    /// The last cap is not redundant. The `close` band is a third of the *last*
    /// band edge held to the sight, and `release_cm` is the *first* one held to
    /// the same sight, so a gun whose inner band is under a third of its outer
    /// reach — nothing the game ships, and an ordinary shape for content to
    /// author — would otherwise let a hurt machine stop in front of a hostile
    /// that `Arena::engage_orders`' own ceiling then refuses to fire at. That
    /// is the parked target 16b cut Q.7 was paid to remove, arrived at by
    /// doctrine instead of by a pilot, and `Arena::withdraw_stand`'s doc says
    /// a unit that cannot shoot is never cornered.
    pub stand_cm: u64,
    /// The furthest this platform fights at at all; `0` is a platform with
    /// nothing loaded, which has no target.
    pub limit_cm: u64,
}

/// [`Ranges`] for one unit under one order.
///
/// **T0's leaky readers, exposed for the twin lane's shadow (17b cut M1a.6) and
/// removed with the reflex in M4.** `#[doc(hidden)]`: a declared surface for
/// `crates/tacsim/tests/twin.rs` and nothing else.
#[doc(hidden)]
pub fn ranges(unit: &UnitState, cell_cm: u16, cfg: &ReflexConfig, order: Option<&Order>) -> Ranges {
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
///
/// **Shared arithmetic, not a leaky reader** (17b cut M1a.6): `pub` because the
/// stock mind's `Approach` and `Engage` call it over the view's held range, so
/// it outlives the reflex's decision half and moves beside the stock mind in
/// 17b cut M4 (see [`Ranges`]); the twin lane's shadow calls it too.
#[must_use]
pub fn stand_off_want(engage_cm: u64, cell_cm: u64, range_cm: u64) -> u64 {
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
            self.adopt_reflex(id, ReflexState::Startup);
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
    ///
    /// **17b cut M1a.4** lifted the formula, verbatim, into `Arena::grid_at`,
    /// which the motor's own tick (`DueKind::MotorTick`) is scheduled on too.
    fn schedule_reflex_tick(&mut self, unit: ObjId) {
        let at = self.grid_at(unit, i64::from(self.arena_cfg.reflex.decide_ms.max(1)));
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
        //
        // **17b cut M1a.1**: so does the motor table, and this hull's row out of
        // it — the row and the table's budget and scratch are two disjoint
        // `&mut`s the drive needs at once — and both go back beside the memory,
        // before any order.
        let mut memory = std::mem::take(&mut self.reflex);
        let mut motors = std::mem::take(&mut self.motors);
        let mut motor = motors.take(unit);
        let plan = self.decide(unit, t_ms, &mut memory, &mut motors, &mut motor, counters);
        let state = memory
            .unit(unit)
            .map_or(ReflexState::Startup, |row| row.state);
        self.reflex = memory;
        motors.put(unit, motor);
        self.motors = motors;
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

    /// **Note that a shot of `shooter`'s had no line** (doc 16 §6's `no line`
    /// edge), which is W's `blocked` and `cover` miss reasons read as a fact about
    /// where the shooter stood when it fired.
    ///
    /// Called from `crate::arena::damage`'s re-validation, so the edge is fed by
    /// the black box's own verdict rather than by a second trace of the
    /// controller's. **16b cut AE.2**: the re-validation traces from the eye the
    /// round left, and calls this only for a shooter that **still stands in the
    /// cell it fired from** — a machine that has moved on since has no line of
    /// that shot's to re-plan.
    pub fn note_no_line(&mut self, shooter: ObjId) {
        if self
            .unit(shooter)
            .is_some_and(|state| state.controller == Controller::Reflex)
            && let Some(row) = self.reflex.units.get_mut(&shooter)
        {
            row.no_line = true;
        }
    }

    /// **Hand a unit to a controller, or take it back** (doc 16 §3.8
    /// "Disconnects"), which is the one thing that moves
    /// [`crate::arena::state::UnitState::controller`] after check-out.
    ///
    /// The tick cadence follows the attribute: a unit handed over joins the stagger
    /// grid from the arena millisecond it was handed over, and a unit handed back
    /// takes its scheduled ticks off the heap with it, so a reclaimed seat costs the
    /// pulse nothing.
    ///
    /// **17b cut M1a.5: the outgoing controller is released on every
    /// transition**, read off the unit before anything is written. From
    /// [`Controller::Reflex`]: its machine and its `ReflexTick`s (today's rule).
    /// From [`Controller::Mind`]: its [`crate::arena::mind::MindState`], its
    /// think waiting to land and every `MindThink` and `MindLand` of the unit.
    /// From **any**: the hull's motor row and every `MotorTick` — a pilot's
    /// `DRIVE` dies with the seat, and a machine's goal with the machine.
    /// Then the incoming one is installed: `Reflex` adopts the machine in
    /// `state` and schedules its tick; `Mind` lays a fresh row down and
    /// schedules its first think; `Pilot` installs nothing. Since 17b cut M1a.1
    /// the motor's readers (`Arena::hold_routeless`, `Arena::note_blocked`)
    /// gate on the motor row and not on the controller, which is why no path
    /// that moves the controller may leave one behind.
    ///
    /// A unit already under `controller` keeps everything it has: a second
    /// `Released` still points a machine at the entry zone, but schedules no
    /// second tick.
    pub fn set_controller(&mut self, unit: ObjId, controller: Controller, state: ReflexState) {
        let was = self.unit(unit).map(|slot| slot.controller);
        let Some(was) = was else {
            return;
        };
        if was == controller {
            if controller == Controller::Reflex {
                self.adopt_reflex(unit, state);
            }
            return;
        }
        // The outgoing controller's rows and cadence.
        match was {
            Controller::Reflex => {
                self.reflex.release(unit);
                self.due.retain(
                    |kind| !matches!(kind, DueKind::ReflexTick { unit: on, .. } if *on == unit),
                );
            }
            Controller::Mind => self.release_mind(unit),
            Controller::Pilot => {}
        }
        self.motors.release(unit);
        self.due
            .retain(|kind| !matches!(kind, DueKind::MotorTick { unit: on } if *on == unit));
        if let Some(slot) = self.unit_mut(unit) {
            slot.controller = controller;
        }
        // The incoming one.
        match controller {
            Controller::Reflex => {
                self.adopt_reflex(unit, state);
                self.schedule_reflex_tick(unit);
            }
            Controller::Mind => self.adopt_mind(unit),
            Controller::Pilot => {}
        }
    }

    /// **Hand a hull to the reflex controller in a named state** (17b cut M1a.1):
    /// [`ReflexMemory::adopt`] for the machine, and the hull's [`MotorRow`] for
    /// the drive — laid down if it has none, and, when the machine was pointed at
    /// a new state, cleared of the goal, the route, the search stamp, the
    /// remembered stops and the stop memory exactly as `adopt` cleared them while
    /// they were the machine's own fields. The waypoint pair, `searched_from` and
    /// the Hold retry clock are kept, as they were.
    ///
    /// The row is laid down on every adoption and not only a changing one, so
    /// every hull the controller flies has a row from its check-out: that is
    /// what the motor's readers ([`Arena::hold_routeless`], [`Arena::note_blocked`])
    /// gate on now in place of the controller.
    pub fn adopt_reflex(&mut self, unit: ObjId, state: ReflexState) {
        let changed = self.reflex.adopt(unit, state);
        let motor = self.motors.ensure(unit);
        if changed {
            motor.goal = None;
            motor.route.clear();
            motor.searched_ms = None;
            motor.avoid.clear();
            // **16b cut AI.1**: and no remembered stop — the goal it was keyed
            // on has just been dropped.
            motor.refused = None;
        }
    }

    /// **Take a hull back from the reflex controller** (17b cut M1a.1): its
    /// machine and its [`MotorRow`] both go — a seat reclaimed is steered by its
    /// pilot's `HEADING` and `SPEED`, which no motor row drives.
    pub fn release_reflex(&mut self, unit: ObjId) {
        self.reflex.release(unit);
        self.motors.release(unit);
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
    ///
    /// **17b cut M1a.1**: `motor` is this hull's [`MotorRow`], taken out of
    /// `motors` by [`Arena::reflex_tick`]; every goal this function and its
    /// pickers write is written there, and the drive spends `motors`' budget.
    #[allow(clippy::too_many_lines)]
    fn decide(
        &self,
        id: ObjId,
        t_ms: i64,
        memory: &mut ReflexMemory,
        motors: &mut MotorTable,
        motor: &mut MotorRow,
        counters: &mut ReflexCounters,
    ) -> ReflexPlan {
        let mut plan = ReflexPlan::default();
        let Some(unit) = self.unit(id) else {
            return plan;
        };
        let mut row = memory.unit(id).cloned().unwrap_or_default();
        // **16b cut Q.7**: remembered stops age out, at the top of the tick —
        // [`MotorRow::age_avoid`], the sweep that stood here, lifted verbatim by
        // 17b cut M1a.1 so a row no decision runs can age too.
        motor.age_avoid(t_ms);

        // ---- the `Disabled` edge, before anything else ----
        //
        // Doc 16 §3.3: a wreck is no longer the subject of any order, and doc 10
        // 3A's disabled precedes destroyed. Either way the machine stops.
        //
        // 17b cut M1a.1: the whole row was reset here, so the motor row is too.
        if unit.limits.wreck || unit.disabled {
            row = ReflexUnit {
                state: ReflexState::Disabled,
                ..ReflexUnit::default()
            };
            *motor = MotorRow::default();
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
        //
        // **16b cut Z.2**: that restoration is `ReflexUnit::drop_to_doctrine`,
        // the one way of dropping to doctrine, which a pilot's `RELEASE` shares.
        if let Some(standing) = row.order.as_ref()
            && standing.expired_at(t_ms)
        {
            let squad = standing.order.squad;
            let objective = standing.order.objective.as_str();
            row.drop_to_doctrine(self.reflex_health_permille(id) >= memory.cfg.withdraw_permille);
            // 17b cut M1a.1: and the motor's half of the restoration, on the row
            // in hand — what `MotorTable::forget_goal` clears.
            motor.goal = None;
            motor.route.clear();
            motor.refused = None;
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
            // **16b cut O.1 (16c N11; plan item 33): an `Advance` into the hold
            // square ends for a unit when it arrives inside the square**, through
            // the same `plan.expired` path and the same one `order_expired` line
            // as AA.1's arm above. The live commander on dev1 always answers an
            // order (its forced tool has no "no order"), and `advance` to the key
            // spot is the reply it is likely to give; left standing, it would park
            // the machine on arrival with the hostile edges suppressed for the
            // order's whole TTL — a holder that cannot shoot back. Ended here, the
            // unit is on doctrine from this tick, `Patrol`'s list hands it to
            // `Arena::hold_goal` (goal `here`, speed 0), and the squad's next
            // signal says `OrderExpired` with the objective in its brief.
            //
            // **Arrival is either of two readings**: the unit's cell inside the
            // square, or the unit within [`ARRIVE_CELLS`] of the ordered cell —
            // `Arena::follow`'s own test, at which it stops. An ordered cell on
            // the square's edge is reached from outside one cell short of it, and
            // with the first reading alone that machine would stand there, out of
            // the square, under the order for its whole TTL; ended here instead,
            // `hold_goal` drives it the last cells in.
            Some(order)
                if matches!(
                    order.objective,
                    Objective::Advance { to } if self.advance_reached_the_hold(to, self.definition.terrain.cell_of(&unit.pose))
                ) =>
            {
                row.order = None;
                plan.expired = Some((order.squad, order.objective.as_str()));
                None
            }
            // **16b cut Z.2 (16c N6)**: a `Support` whose charge has left the
            // arena, or whose charge this unit's own contact reads dead
            // ([`Arena::contact_reads_dead`] — what the unit knows, never the
            // charge's live wreck flag), is an order with nothing to keep station
            // on. It ends for THIS unit through `B-41`'s route, beside AA.1's: one
            // `order_expired` line, the squad marked, so a squad whose charge died
            // drops to doctrine and its commander is told. A charge the unit
            // merely cannot see is not this: it holds and fights where it stands
            // (the leash block below).
            Some(order) if self.support_charge_lost(id, &order) => {
                row.order = None;
                plan.expired = Some((order.squad, order.objective.as_str()));
                None
            }
            other => other,
        };

        // ---- the inert rule (16b cut O.3; 16c N13; plan item 7) ----
        //
        // **A unit that cannot move and carries no weapon is inert to this
        // controller**: it starts up, and then it does nothing at all. It is the
        // general rule for any such hull — `tacsim.unit.target_mk1`, the range's
        // target, is the one the game ships — and it sits beside AA.1's immobile
        // reading ([`UnitState::cannot_close`]), which is about a machine that
        // cannot close but can still shoot: that one holds and fights from where
        // it stands; this one has nothing to hold with.
        //
        // Why a rule and not the ordinary path: nothing below would leave such a
        // unit alone. `follow` plans for any goal it is handed, and `cannot_close`
        // only moves `Approach`'s closing edge — it does not stop `Patrol`'s list
        // choosing a search, a hunt, a hold spot or a waypoint. A target standing
        // on the machine slots would otherwise spend a search every `repath_ms` of
        // every window on waypoints it can never reach, and hunt contacts it can
        // never shoot. So once it has power it is parked in `Patrol` with no goal,
        // no route, no target, no hunt and no plot, and nothing else is asked: no
        // search, no path query, no intent — and it never reaches O.1's
        // `hold_goal`, the `Approach` rule or the `Advance` clause.
        //
        // A **cold** inert unit takes the `Power::Off` arm below as any machine
        // does, so it comes online and a thermal sight reads it hot. The
        // predicate is two reads of the custody's constants and one `is_empty`:
        // on a lane that fields only armed types it is false at its first term.
        let inert = unit.weapons.is_empty() && unit.mobility.max_speed_cms == 0;
        if inert && matches!(unit.power, Power::Online | Power::Starting) {
            row.state = ReflexState::Patrol;
            motor.goal = None;
            motor.route.clear();
            row.target = None;
            row.hunt = None;
            row.last_plot = None;
            memory.units.insert(id, row);
            return plan;
        }

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
                    motor.goal = None;
                    motor.route.clear();
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
                motor.goal = None;
                motor.route.clear();
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
        //   `engage`, which is the whole reason there are four movement-and-fight
        //   objectives (and, since 16b cut Z.2, two squad roles) and not
        //   two (an `advance` held by a machine that cannot close never reaches
        //   here: it ended above the power arms, 16b cut AA.1);
        // - `engage` changes nothing about the states and everything about the
        //   choice inside them: the utility score prefers the hinted contact.
        //
        // **16b cut Z.2 (16c N6)**: `screen` is a movement order in this sense —
        // it never pursues, and its shooting is the `Patrol` arm's stand — and
        // `support` is `engage`'s arm: the machine fights, preferring what its
        // charge fights (`ordered_target`), under the leash below.
        if let Some(order) = order.as_ref() {
            match order.objective {
                Objective::Withdraw => {
                    if row.state != ReflexState::Withdraw {
                        row.state = ReflexState::Withdraw;
                        row.target = None;
                        motor.goal = None;
                        motor.route.clear();
                        // **16b cut E.4**: a machine breaking off does not search.
                        // Its goal is its own entry zone, and the last place it
                        // saw the enemy is the one place it must not drive to.
                        row.last_plot = None;
                    }
                }
                Objective::Hold | Objective::Advance { .. } | Objective::Screen { .. } => {
                    if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                        row.state = ReflexState::Patrol;
                        motor.goal = None;
                        motor.route.clear();
                    }
                    row.target = None;
                }
                Objective::Engage { .. } | Objective::Support { .. } => {}
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
            motor.goal = None;
            motor.route.clear();
            // **16b cut E.4**: `Withdraw` does not search — see the order's own
            // arm above for why.
            row.last_plot = None;
        }

        // ---- the hostile edges ----
        //
        // 16b cut N.2: a movement order suppresses them entirely (see above), so an
        // ordered squad walks past a contact rather than turning on it. **16b cut
        // Z.2**: so does a screen, which shoots from where it stands and never
        // gives chase; a support pursues, under its leash.
        //
        // **The leash's reading, taken here** (16b cut Z.2; 16c N6): a supporter
        // whose charge is not itself reads where its **own** contact on the
        // charge last saw it — the plot, never the charge's live pose (doc 16
        // §1, 16b cut F.3's rule once more). `None` is a supporter that cannot
        // see its charge; the stand below fights for it. Read before the hostile
        // edges so that a supporter the leash is about to override does not
        // start a chase first: off the leash, or blind to its charge, the edges
        // are not run at all, which is what the leash block would undo anyway —
        // but undone on every tick it would throw the rejoining route away with
        // the target the edge re-took.
        let leash: Option<Option<SeenPose>> = match order.as_ref().map(|order| order.objective) {
            Some(Objective::Support { of })
                if of != id
                    && matches!(
                        row.state,
                        ReflexState::Patrol | ReflexState::Approach | ReflexState::Engage
                    ) =>
            {
                Some(
                    self.contacts
                        .of(id)
                        .find(|(_, held)| held.target == of)
                        .map(|(_, held)| held.seen_at),
                )
            }
            _ => None,
        };
        let cell_cm = self.definition.terrain.cell_cm();
        let leash_cm = u64::from(SUPPORT_LEASH_CELLS).saturating_mul(u64::from(cell_cm));
        let station_cm = u64::from(SUPPORT_STATION_CELLS).saturating_mul(u64::from(cell_cm));
        let off_leash = match leash {
            Some(None) => true,
            Some(Some(plot)) => plot.range_from(&unit.pose) > leash_cm,
            None => false,
        };
        let may_pursue = !off_leash
            && !matches!(
                order.as_ref().map(|order| order.objective),
                Some(Objective::Hold | Objective::Advance { .. } | Objective::Screen { .. })
            );
        if may_pursue
            && matches!(
                row.state,
                ReflexState::Patrol | ReflexState::Approach | ReflexState::Engage
            )
        {
            let prefer = self.ordered_target(id, order.as_ref(), memory);
            match self.best_hostile(id, memory, health, prefer) {
                Some(found) => {
                    if row.target != Some(found) {
                        row.target = Some(found);
                        motor.goal = None;
                        motor.route.clear();
                    }
                    if row.state == ReflexState::Patrol {
                        row.state = ReflexState::Approach;
                    }
                }
                None => {
                    row.target = None;
                    if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                        row.state = ReflexState::Patrol;
                        motor.goal = None;
                        motor.route.clear();
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
            motor.goal = None;
            motor.route.clear();
            if row.state == ReflexState::Engage {
                row.state = ReflexState::Approach;
            }
        }

        // ---- the state's own goal and orders ----
        let here = self.definition.terrain.cell_of(&unit.pose);
        // **16b cut E.1**: the three distances this tick fights at, computed once
        // from the unit, the doctrine and the order — see [`Ranges`] for what
        // each one is and why the doctrine band is applied after the order's cap.
        let ranges = ranges(unit, cell_cm, &memory.cfg, order.as_ref());
        // **16b cut E.4**: the cell a `Withdraw` that stood and fought turned
        // onto. `Withdraw` clears `row.target` on entry and `withdraw_stand`
        // chooses its own candidate without writing one back, so the stand is the
        // one plot this tick holds that `row.target` cannot name. **16b cut
        // Z.2**: a screen's stand and a blind supporter's write it the same way.
        let mut stood_cell: Option<(u16, u16)> = None;

        // ---- the leash (16b cut Z.2; 16c N6) ----
        //
        // A supporter whose charge is not itself, in `Patrol`, `Approach` or
        // `Engage` (a hurt one in `Withdraw` goes home: the health edge outranks
        // the order, as it does `engage`'s). Two of its three cases take the tick
        // over and skip the state's own arm below; the third falls through.
        let mut leashed = false;
        // Case (a)'s two readers: blind to the charge, or past the leash with no
        // way of closing it.
        let stands = match leash {
            Some(None) => true,
            Some(Some(_)) => off_leash && unit.cannot_close(),
            None => false,
        };
        match leash {
            // (a) **A supporter that cannot see its charge holds where it stands
            // and fights from there.** It does not go looking — the charge's
            // position is its own plot, and it has none — and it does not stand
            // idle: the stand below takes the best classified hostile inside
            // `release_cm` with a line, over a full arc.
            //
            // **And a supporter past its leash that cannot close** (16b cut Z.2's
            // review; the `B-76` class, as 16b cut AA.1 closed it for `Advance`):
            // a destroyed engine, or a hull with no speed
            // ([`UnitState::cannot_close`]), will never rejoin, so case (b)'s
            // rejoining leg — which fires nothing — would hold it silent for the
            // order's whole life while a hostile shot it at point blank, and being
            // armed it would hold its side up. It takes this same stand instead:
            // where it is, over a full arc, preferring its charge's designation
            // (`ordered_target`) inside `release_cm`, which is what an immobile
            // `Approach` does as well (`cannot_close` counts as arrived there).
            //
            // The two are one arm through `stands`, and (b) below never sees them.
            _ if stands => {
                leashed = true;
                if matches!(row.state, ReflexState::Approach | ReflexState::Engage) {
                    row.state = ReflexState::Patrol;
                }
                row.target = None;
                motor.goal = here;
                motor.route.clear();
                plan.speed_cms = Some(0);
                row.last_plot = None;
                row.hunt = None;
                row.unheld_since_ms = Some(t_ms);
                if let Some((contact, plot)) =
                    self.stand(id, memory, health, &ranges, None, order.as_ref(), counters)
                {
                    stood_cell = plot.cell(&self.definition.terrain);
                    plan.heading_mdeg = Some(plot.bearing_from(&unit.pose));
                    self.engage_orders(id, contact, order.as_ref(), &ranges, t_ms, &mut plan);
                }
            }
            // (b) **Past the leash, the leash wins over the chase**: back to
            // `Patrol` with the target dropped, the goal the station cell off the
            // supporter's own plot of its charge. **A rejoining supporter fires
            // nothing until it is back inside the leash** — the state arm, whose
            // `engage_orders` is the only trigger it has, is skipped — which is
            // a recorded residual of the cut (a supporter that fires while it
            // drives is not built, as `Approach`'s is not).
            Some(Some(plot)) if off_leash => {
                leashed = true;
                row.state = ReflexState::Patrol;
                row.target = None;
                row.last_plot = None;
                row.hunt = None;
                row.unheld_since_ms = Some(t_ms);
                let goal = self.stand_off_cell(id, &plot, station_cm).or(here);
                if motor.goal != goal {
                    motor.goal = goal;
                    motor.route.clear();
                }
            }
            // (c) inside the leash: the state's own arm, with the charge's
            // designation preferred (`ordered_target`) and `Patrol`'s station.
            // (`Some(None)` always `stands`, and is here only for the match.)
            Some(Some(_) | None) | None => {}
        }
        // The charge's plot when the supporter is inside its leash, for the
        // `Patrol` arm's station.
        let station_on = if leashed { None } else { leash.flatten() };
        if !leashed {
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
                    //
                    // **16b cut Z.2**: a screen is a movement order in exactly this
                    // sense, and so is a supporter keeping station on a charge it can
                    // see — its goal is the station, not a plot of its own or the far
                    // zone.
                    let movement = matches!(
                        order.as_ref().map(|order| order.objective),
                        Some(
                            Objective::Advance { .. } | Objective::Hold | Objective::Screen { .. }
                        )
                    ) || station_on.is_some();
                    if movement {
                        row.last_plot = None;
                        row.hunt = None;
                        row.unheld_since_ms = Some(t_ms);
                    }
                    match order.as_ref().map(|order| order.objective) {
                        Some(Objective::Advance { to }) => {
                            if motor.goal != Some(to) {
                                motor.goal = Some(to);
                                motor.route.clear();
                            }
                        }
                        Some(Objective::Hold) => {
                            motor.goal = here;
                            motor.route.clear();
                            plan.speed_cms = Some(0);
                        }
                        // **16b cut Z.2 (16c N6): the screen.** Parked where it
                        // stands and faced onto the order's bearing, and the stand
                        // takes the best classified hostile inside the arc and inside
                        // `release_cm` with a line: turned onto it, locked, and fired
                        // on unless the order holds fire (`engage_orders` honours
                        // `hold_fire`). The machine never leaves `Patrol` for it and
                        // never moves for it.
                        Some(Objective::Screen { facing_mdeg }) => {
                            motor.goal = here;
                            motor.route.clear();
                            plan.speed_cms = Some(0);
                            plan.heading_mdeg = Some(facing_mdeg % 360_000);
                            if let Some((contact, plot)) = self.stand(
                                id,
                                memory,
                                health,
                                &ranges,
                                Some((facing_mdeg, SCREEN_HALF_ARC_MDEG)),
                                order.as_ref(),
                                counters,
                            ) {
                                stood_cell = plot.cell(&self.definition.terrain);
                                plan.heading_mdeg = Some(plot.bearing_from(&unit.pose));
                                self.engage_orders(
                                    id,
                                    contact,
                                    order.as_ref(),
                                    &ranges,
                                    t_ms,
                                    &mut plan,
                                );
                            }
                        }
                        // **16b cut Z.2 (16c N6): a supporter idle inside its leash
                        // keeps station.** The station cell — `SUPPORT_STATION_CELLS`
                        // back from its own plot of the charge toward itself — is the
                        // goal until the unit stands inside that range of the plot,
                        // and then it holds. The station is what is reached, not the
                        // leash's edge: a hold at the first tick inside the leash
                        // would stop the supporter there for good, because `follow`
                        // runs only when no speed is ordered.
                        //
                        // One step more than the station cell: `follow` stops within
                        // `ARRIVE_CELLS` of a goal — and brakes to do it — so a unit
                        // that has arrived at the station cell may still stand a cell
                        // or so outside the station range, where the station cell
                        // would hold it for ever. Arrived there and still outside, it
                        // closes on the plot's own cell, which `follow` brakes short
                        // of, and the first tick inside the range holds it.
                        Some(Objective::Support { .. }) if station_on.is_some() => {
                            if let Some(plot) = station_on {
                                if plot.within_cm(&unit.pose, station_cm) {
                                    motor.goal = here;
                                    motor.route.clear();
                                    plan.speed_cms = Some(0);
                                } else {
                                    let station = self.stand_off_cell(id, &plot, station_cm);
                                    let at_station = match (here, station) {
                                        (Some(here), Some(cell)) => {
                                            here.0.abs_diff(cell.0).max(here.1.abs_diff(cell.1))
                                                <= ARRIVE_CELLS
                                        }
                                        _ => false,
                                    };
                                    let goal = if at_station {
                                        plot.cell(&self.definition.terrain).or(station)
                                    } else {
                                        station
                                    }
                                    .or(here);
                                    if motor.goal != goal {
                                        motor.goal = goal;
                                        motor.route.clear();
                                    }
                                }
                            }
                        }
                        // **16b cut E.4 (16c `B-04`)**: with no movement order the
                        // goal is the search, then the hunt, then the waypoint walk.
                        // Each answers `true` when it has taken the goal, so the one
                        // below it is not asked; a movement order above never reaches
                        // here, and has just cleared both clocks' goals, so neither
                        // survives one.
                        //
                        // **16b cut O.1 (16c N11)**: in a Hold match the key spot
                        // takes the hunt's place and the walk's — see
                        // [`ReflexState::Patrol`] and [`Arena::hold_goal`] — and
                        // the walk is asked only while the spot is given up.
                        _ => {
                            if !self.search_goal(t_ms, &mut row, motor, here, &memory.cfg) {
                                if self.hold.is_some() {
                                    if !self.hold_goal(t_ms, unit, &mut row, motor, here, &mut plan)
                                    {
                                        self.patrol_step(id, motor, here);
                                    }
                                } else if !self.hunt_goal(id, t_ms, &mut row, motor, here, memory) {
                                    self.patrol_step(id, motor, here);
                                }
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
                        //
                        // **16b cut O.1 (16c N11; plan item 32): a machine inside
                        // the hold square fights from it.** It takes
                        // `cannot_close`'s two readings — `release_cm` as its edge
                        // into `Engage`, and `here` as its goal outside that edge —
                        // and one more: `here` inside the edge with no line, where
                        // a machine that can drive would otherwise close on the
                        // plot and leave the square. So it fires at anything with a
                        // line inside its reach without leaving the radius, and
                        // `Engage`'s `goal = here` keeps it there. What is NOT
                        // changed, recorded for the owner in Q-O1: a machine
                        // OUTSIDE the square — an attacker
                        // sighting a holder — still stops at `engage_cm` (42 cells
                        // for the stock tank) and never drives in to contest it.
                        let inside = self.hold.as_ref().is_some_and(|hold| {
                            here.is_some_and(|cell| hold.square_contains(cell))
                        });
                        let close_edge = if unit.cannot_close() || inside {
                            ranges.release_cm
                        } else {
                            ranges.engage_cm
                        };
                        if ranges.limit_cm != 0 && range <= close_edge.max(1) {
                            if self.reflex_has_line(id, target, counters) {
                                row.state = ReflexState::Engage;
                                motor.goal = None;
                                motor.route.clear();
                                self.engage_orders(
                                    id,
                                    contact,
                                    order.as_ref(),
                                    &ranges,
                                    t_ms,
                                    &mut plan,
                                );
                            } else if inside {
                                // **16b cut O.1 (plan item 32)**: inside the edge
                                // with no line, the holder does NOT close on the
                                // plot, which would walk it off the square cell
                                // by cell and hand the spot to whoever is left
                                // on it. It stands and waits for a line — the
                                // reading `cannot_close` gets for nothing, since
                                // a machine that cannot drive goes nowhere
                                // whatever its goal says.
                                motor.goal = here;
                            } else {
                                motor.goal = plot
                                    .and_then(|plot| plot.cell(&self.definition.terrain))
                                    .or(here);
                            }
                        } else if inside {
                            // 16b cut O.1: on the spot, it stands and waits for
                            // the hostile to come inside its reach.
                            motor.goal = here;
                        } else {
                            let want =
                                stand_off_want(ranges.engage_cm, u64::from(cell_cm.max(1)), range);
                            motor.goal = plot
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
                            motor.goal = plot
                                .and_then(|plot| self.stand_off_cell(id, &plot, want))
                                .or(here);
                        } else {
                            // Holding the band: stop, turn onto the plot, shoot.
                            motor.goal = here;
                            motor.route.clear();
                            plan.speed_cms = Some(0);
                            plan.heading_mdeg = plot.map(|plot| plot.bearing_from(&unit.pose));
                            self.engage_orders(
                                id,
                                contact,
                                order.as_ref(),
                                &ranges,
                                t_ms,
                                &mut plan,
                            );
                        }
                    }
                }
                // **16b cut Q.7**: `Withdraw` is no longer terminal. It has two new
                // edges inside it and one out of it, and none of them is a state flip
                // on a live line — see [`ReflexUnit::stand_until_ms`] for why.
                ReflexState::Withdraw => {
                    if motor.goal.is_none() {
                        motor.goal = self.entry_cell(id).or(here);
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
                        || match (here, motor.goal) {
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
                                motor.goal = None;
                                motor.route.clear();
                                row.quiet_since_ms = None;
                            }
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
            // **17b cut M1a.1**: the drive is the motor's (`Arena::follow`, under
            // the reflex controller's own [`Fallback::Waypoint`]), and what it
            // orders is copied onto this tick's plan — only what it wrote, which
            // is what it wrote onto the plan itself until this cut.
            let mut steer = MotorPlan::default();
            let outcome = self.follow(
                id,
                t_ms,
                motors,
                motor,
                here,
                Fallback::Waypoint,
                counters,
                &mut steer,
            );
            if let Some(mdeg) = steer.heading_mdeg {
                plan.heading_mdeg = Some(mdeg);
            }
            if let Some(cms) = steer.speed_cms {
                plan.speed_cms = Some(cms);
            }
            // **The `unreachable` edge's decision half** (17b cut M1a.1). The
            // motor answers the goal no route reached and has already fallen
            // back to a waypoint; what the edge did to THIS row's fields it now
            // does here, from that answer, in the order it did it. The order
            // compared against is the one this tick decided under — the one the
            // squad filter above already applied — because a search that fails
            // for the order's OWN cell ends the order for this unit, and nothing
            // else does; a unit whose roster moved under the order, and which is
            // therefore obeying nothing, drops nothing either.
            if let Some(failed) = outcome.unreachable {
                // **16b cut E.2 (16c `B-41`)**: and when the goal that
                // failed is the live order's own `Advance` cell, the
                // **order is over for this unit**.
                //
                // Before this cut the fallback below took, and the
                // `Patrol` arm re-asserted the ordered cell on the very
                // next tick (`motor.goal != Some(to)`), so the fallback
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
                // **Only the ordered cell** (and, since 16b cut Z.2, a
                // supporter's station: see below). A stand-off cell under
                // an `Engage` order, the entry cell under a `Withdraw`
                // and a patrol node all fail through this same arm and
                // drop nothing: they fall back exactly as they did.
                //
                // What it costs, stated: a commander that re-sends the
                // same unreachable cell buys one failed search per unit
                // per order, which the deliberation floor bounds at one
                // per `deliberate_min_ms`; and a unit whose route was
                // sealed only by a remembered stop
                // ([`MotorRow::avoid`], which ages out after
                // [`REFLEX_AVOID_TTL_MS`]) loses the rest of that order
                // where a retry thirty seconds later would have kept it.
                // The commander is told within one signal either way,
                // which is the better half of that trade.
                if let Some(order) = order.as_ref()
                    && let Objective::Advance { to } = order.objective
                    && failed == to
                {
                    row.order = None;
                    plan.expired = Some((order.squad, order.objective.as_str()));
                }
                // **16b cut Z.2's review (16c N6; `B-41`'s lesson once
                // more)**: the station of a `Support` whose charge is
                // another unit is the order's own goal in exactly the
                // sense the `Advance` cell is. A supporter in `Patrol`
                // under one steers at nothing else — the leash's case (b)
                // and the `Patrol` arm's station leg are the only goals
                // it is given there, and both re-assert the station on
                // the next tick, so the fallback below would never hold
                // — and case (b) fires nothing while it rejoins. A
                // station no route reaches (the charge across ground
                // this hull's class refuses) would therefore be a silent
                // supporter and a failed search every `repath_ms` for
                // the order's whole life. It ends here for this unit,
                // through the same `plan.expired`: one `order_expired`
                // line naming `support`, the squad marked, the commander
                // told, and from the next tick the unit fights on
                // doctrine. Under `Approach` or `Engage` the goal is a
                // stand-off cell or a plot of the unit's own target,
                // which fall back as they always did.
                if let Some(order) = order.as_ref()
                    && let Objective::Support { of } = order.objective
                    && of != id
                    && row.state == ReflexState::Patrol
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
                if row.last_plot.is_some_and(|(cell, _)| failed == cell) {
                    row.last_plot = None;
                }
                if row.hunt.is_some_and(|cell| failed == cell) {
                    row.hunt = None;
                    row.unheld_since_ms = Some(t_ms);
                }
            }
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
        motor: &mut MotorRow,
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
        if motor.goal != Some(cell) {
            motor.goal = Some(cell);
            motor.route.clear();
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
    /// its own row and, since 16b cut AI.1, its fan keeps each new hunt cell
    /// [`HUNT_FAN_ROWS`] (Chebyshev) clear of every hunt cell the side already
    /// holds — read here off the side's own reflex rows, in the order the ticks
    /// fall — so two machines of one side, a column on one road row or two rows
    /// a cell apart, do not arrive at one cell and turn back into each other
    /// while the zone has room for the fan; `MotorTable::may_search`
    /// rations the searches, and 16b cut E.3's hold is what a column at a ford
    /// needs.
    ///
    /// A far zone that answers `None` — no other side has one, or nothing in it
    /// is passable — restarts the clock rather than leaving it run out, so the
    /// scan is paid once per `hunt_after_ms` and not once per tick. That is the
    /// honest cost of an answer that is honestly nothing.
    ///
    /// **17b cut M1a.1**: the goal it takes, and the waypoint it forgets on
    /// arrival, are the hull's [`MotorRow`]'s; the crossing and its clock stay
    /// this row's, and so does `taken` — read off the side's own machines, which
    /// is the decision's knowledge and not the motor's.
    fn hunt_goal(
        &self,
        id: ObjId,
        t_ms: i64,
        row: &mut ReflexUnit,
        motor: &mut MotorRow,
        here: Option<(u16, u16)>,
        memory: &ReflexMemory,
    ) -> bool {
        let cfg = &memory.cfg;
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
                // **here**: [`Arena::patrol_step`] keeps a node it holds and asks
                // the graph for the nearest only when it holds none, so without
                // this the machine would turn round at the far edge and drive the
                // whole map back to a node on its own side.
                row.hunt = None;
                row.unheld_since_ms = Some(t_ms);
                motor.waypoint = None;
                motor.from_waypoint = None;
                return false;
            }
            if motor.goal != Some(cell) {
                motor.goal = Some(cell);
                motor.route.clear();
            }
            return true;
        }
        let since = *row.unheld_since_ms.get_or_insert(t_ms);
        if cfg.hunt_after_ms == 0 || t_ms.saturating_sub(since) < i64::from(cfg.hunt_after_ms) {
            return false;
        }
        // **16b cut AI.1**: the hunt cells the rest of this side holds right
        // now — its own reflex rows, hashed, read at this tick in `(t_ms, seq)`
        // order, so a side's hunters are handed cells one after another in the
        // order their ticks fall, at every tempo. A side knows its own
        // machines' goals; nothing here reads another side's.
        let team = self.unit(id).map(|unit| unit.team);
        let taken: Vec<(u16, u16)> = memory
            .units
            .iter()
            .filter(|(other, _)| **other != id)
            .filter(|(other, _)| self.unit(**other).map(|unit| unit.team) == team)
            .filter_map(|(_, other)| other.hunt)
            .collect();
        let Some(cell) = self.far_entry_cell(id, &taken) else {
            row.unheld_since_ms = Some(t_ms);
            return false;
        };
        row.hunt = Some(cell);
        if motor.goal != Some(cell) {
            motor.goal = Some(cell);
            motor.route.clear();
        }
        true
    }

    /// **`Patrol`'s Hold goal: the key spot** (16b cut O.1; 16c N11; plan items
    /// 2, 16, 34). Answers whether it took the goal; asked only in a Hold match,
    /// in place of both [`Arena::hunt_goal`] and [`Arena::patrol_step`].
    ///
    /// It is **doctrine's own fallback goal** and not a standing
    /// `Objective::Advance` written by the arena: a live `Advance` suppresses the
    /// hostile edges for its whole TTL and arrival does not end it, so a machine
    /// told to advance to the spot would park there unable to shoot back. As a
    /// goal it leaves the hostile edges running, and a hostile sighted pre-empts
    /// it without this being asked at all.
    ///
    /// * **Declines** while [`MotorRow::hold_retry_ms`] is in the future — a
    ///   spot no route reached, given up by [`Arena::follow`]'s `unreachable`
    ///   edge, so the list falls through to the waypoint walk and the fallback
    ///   waypoint holds — and clears the clock once past.
    /// * **Arrived** — a machine that [`UnitState::cannot_close`], or one whose
    ///   cell is within `radius_cells − 1` of the spot (Chebyshev): the goal is
    ///   `here`, the route is cleared and the speed ordered is `0`, as
    ///   `Objective::Hold` parks a machine. Anywhere inside and not the spot's
    ///   own cell, because presence is all a hold needs and one cell with
    ///   [`ARRIVE_CELLS`] of one cannot seat a side; one cell short of the edge,
    ///   so a machine braking onto the square does not stop astride it.
    /// * **Otherwise** the goal is the spot itself (the route cleared on a
    ///   change), and [`Arena::follow`] drives it there.
    ///
    /// Either way the hunt is dropped and its clock restarted: a machine with a
    /// spot to hold has no business crossing to the far zone.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hold_goal(
        &self,
        t_ms: i64,
        unit: &UnitState,
        row: &mut ReflexUnit,
        motor: &mut MotorRow,
        here: Option<(u16, u16)>,
        plan: &mut ReflexPlan,
    ) -> bool {
        let Some(hold) = self.hold.as_ref() else {
            return false;
        };
        if motor.hold_retry_ms.is_some_and(|until| t_ms < until) {
            return false;
        }
        motor.hold_retry_ms = None;
        // 17b cut M1a.1: the arrival arithmetic is the motor's kernel now; the
        // retry clock above stays the decision's.
        if self.hold_arrived(unit.id, here) {
            motor.goal = here;
            motor.route.clear();
            plan.speed_cms = Some(0);
        } else if motor.goal != Some(hold.spot) {
            motor.goal = Some(hold.spot);
            motor.route.clear();
        }
        row.hunt = None;
        row.unheld_since_ms = Some(t_ms);
        true
    }

    /// **Whether an `Advance { to }` has reached the Hold square** (16b cut O.1;
    /// plan item 33): `to` is inside the square, and the unit's cell `here` is
    /// either inside it too or within [`ARRIVE_CELLS`] of `to` — where
    /// [`Arena::follow`] stops it. `false` on a skirmish, off the window, and for
    /// an ordered cell outside the square (that `Advance` is the commander's
    /// own business and runs its course).
    fn advance_reached_the_hold(&self, to: (u16, u16), here: Option<(u16, u16)>) -> bool {
        let Some(hold) = self.hold.as_ref() else {
            return false;
        };
        hold.square_contains(to)
            && here.is_some_and(|cell| {
                hold.square_contains(cell)
                    || cell.0.abs_diff(to.0).max(cell.1.abs_diff(to.1)) <= ARRIVE_CELLS
            })
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

    /// **A stand: what a unit that will not move fights from where it is** (16b
    /// cut Z.2; 16c N6) — a screen over its arc, and a supporter that cannot see
    /// its charge over a full one — and the plot to turn onto.
    ///
    /// [`Arena::withdraw_stand`]'s shape: `None` for a platform with nothing left
    /// to fire ([`is_dry`]); otherwise the utility score's own best classified
    /// hostile ([`Arena::best_hostile_where`], the one candidate loop), **restricted
    /// to** contacts whose plot lies inside `ranges.release_cm` — the ceiling
    /// [`Arena::engage_orders`] fires out to, so a stand never turns onto a
    /// hostile it may not shoot — and, when `sector` is `Some((facing, half))`,
    /// whose plot bears within `half` of `facing` (modulo a full turn); then one
    /// line trace, paid only for the one candidate. The order's own preference
    /// ([`Arena::ordered_target`]) wins inside the restriction exactly as it does
    /// in `Approach`.
    ///
    /// `hold_fire` is deliberately **not** asked here, where `withdraw_stand`
    /// asks it: a stand does not stop a tank in front of a gun (a screen or a
    /// blind supporter is already stopped by its order), so a screen holding fire
    /// still turns and locks — [`Arena::engage_orders`] is what keeps the
    /// trigger still.
    #[allow(clippy::too_many_arguments)]
    fn stand(
        &self,
        id: ObjId,
        memory: &ReflexMemory,
        health_permille: u32,
        ranges: &Ranges,
        sector: Option<(u32, u32)>,
        order: Option<&Order>,
        counters: &mut ReflexCounters,
    ) -> Option<(ContactId, SeenPose)> {
        let unit = self.unit(id)?;
        if is_dry(unit) {
            return None;
        }
        let ceiling = ranges.release_cm.max(1);
        let prefer = self.ordered_target(id, order, memory);
        let (contact, target) =
            self.best_hostile_where(id, memory, health_permille, prefer, |plot| {
                plot.within_cm(&unit.pose, ceiling)
                    && sector.is_none_or(|(facing, half)| {
                        arc_between(plot.bearing_from(&unit.pose), facing) <= half
                    })
            })?;
        let plot = self.plot_of(id, contact)?;
        if !self.reflex_has_line(id, target, counters) {
            return None;
        }
        Some((contact, plot))
    }

    /// **Whether a `Support` order has lost its charge for this unit** (16b cut
    /// Z.2; 16c N6): the charge has left the arena, or this unit's **own**
    /// contact on it reads dead ([`Arena::contact_reads_dead`] — what the unit
    /// knows, never the charge's live wreck flag). A self-charge, and every other
    /// objective, answers `false`; so does a charge the unit merely cannot see.
    fn support_charge_lost(&self, id: ObjId, order: &Order) -> bool {
        let Objective::Support { of } = order.objective else {
            return false;
        };
        if of == id {
            return false;
        }
        if self.unit(of).is_none() {
            return true;
        }
        self.contacts
            .of(id)
            .find(|(_, held)| held.target == of)
            .is_some_and(|(_, held)| self.contact_reads_dead(held))
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

    /// **The object an order points this unit at**, or `None` (16b cut N.2; the
    /// issuer since cut Y.1).
    ///
    /// The hint is written in **the issuer's** contact ids — a pilot's own tank
    /// when a pilot typed `ORDER` ([`Order::issuer`]), or the squad commander's
    /// when the LLM seam wrote it (`issuer: None`), because that is the contact
    /// store the order was written against — and it is resolved there into an
    /// object; the caller then looks for **this** unit's own contact on that
    /// object. A unit that holds nothing on it prefers nothing and takes its
    /// doctrine's best target, which is doc 16 §3.6's observer rule read through an
    /// order: a commander may point, and a tank that cannot see still cannot see.
    ///
    /// A `match` and not `unwrap_or(self.squad_of(id)?.commander)`: that
    /// argument's `?` is eager, and would drop a pilot's hint on a unit that is in
    /// no squad before the issuer were ever looked at.
    ///
    /// # A support prefers what its charge is fighting (16b cut Z.2; 16c N6)
    ///
    /// Under `Objective::Support { of }` with `of` another unit, **the charge's
    /// own designation** is asked first, before the hint: a charge under the
    /// controller answers its machine's target, read off `memory` — the memory
    /// `decide` holds, and **never `self.reflex`**, which `Arena::reflex_tick`
    /// has taken out of the arena for the length of the decision and is
    /// therefore empty here — and a charge under a pilot answers the object its
    /// lock resolves to in the pilot's own store. A charge that designates
    /// nothing falls through to the hint. What the supporter does with the answer
    /// is what it does with a hint: it looks for **its own** contact on that
    /// object, and a supporter that holds nothing on it prefers nothing (doc 16
    /// §3.6 read through an order, again).
    ///
    /// A charge's designation is the side's own data-link knowledge — the same
    /// class of fact the mesh carries between teammates — and never a live pose:
    /// it names *which* object, and every range and bearing the supporter then
    /// acts on is its own plot. A unit whose `of` is itself fights on doctrine
    /// and is asked nothing here.
    ///
    /// **T0's leaky readers, exposed for the twin lane's shadow (17b cut M1a.6) and
    /// removed with the reflex in M4.** `#[doc(hidden)]`: a declared surface for
    /// `crates/tacsim/tests/twin.rs` and nothing else.
    #[doc(hidden)]
    pub fn ordered_target(
        &self,
        id: ObjId,
        order: Option<&Order>,
        memory: &ReflexMemory,
    ) -> Option<ObjId> {
        let order = order?;
        if let Objective::Support { of } = order.objective
            && of != id
        {
            let designated = match self.unit(of) {
                Some(charge) if charge.controller == Controller::Reflex => memory
                    .unit(of)
                    .and_then(|row| row.target)
                    .map(|(_, target)| target),
                Some(charge) => charge
                    .lock
                    .and_then(|contact| self.contacts.resolve(of, contact))
                    .map(|held| held.target),
                None => None,
            };
            if designated.is_some() {
                return designated;
            }
        }
        let hint = order.preferred_contact()?;
        let viewer = match order.issuer {
            Some(unit) => unit,
            None => self.squad_of(id)?.commander,
        };
        self.contacts.resolve(viewer, hint).map(|held| held.target)
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
    ///
    /// **T0's leaky readers, exposed for the twin lane's shadow (17b cut M1a.6) and
    /// removed with the reflex in M4.** `#[doc(hidden)]`: a declared surface for
    /// `crates/tacsim/tests/twin.rs` and nothing else.
    #[doc(hidden)]
    pub fn reflex_has_line(&self, id: ObjId, target: ObjId, counters: &mut ReflexCounters) -> bool {
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
        self.best_hostile_where(id, memory, health_permille, prefer, |_| true)
    }

    /// **[`Arena::best_hostile`]'s candidate loop, with a filter** (16b cut Z.2):
    /// the same score over the same candidates, restricted to the contacts whose
    /// plot `admit` accepts — which is how [`Arena::stand`] asks for the best
    /// hostile inside an arc and a range without a second loop that could drift
    /// from the first. `best_hostile` is this with a filter that admits all.
    ///
    /// **T0's leaky readers, exposed for the twin lane's shadow (17b cut M1a.6) and
    /// removed with the reflex in M4.** `#[doc(hidden)]`: a declared surface for
    /// `crates/tacsim/tests/twin.rs` and nothing else.
    #[doc(hidden)]
    pub fn best_hostile_where(
        &self,
        id: ObjId,
        memory: &ReflexMemory,
        health_permille: u32,
        prefer: Option<ObjId>,
        admit: impl Fn(&SeenPose) -> bool,
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
            // **16b cut Z.2**: the caller's restriction, on the plot.
            if !admit(&held.seen_at) {
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
    ///
    /// **17b cut M1a.5** (doc 17 §4, D-UM-12): the body is
    /// [`Arena::apply_machine_intents`] now, which a mind's landing calls too;
    /// this builds the plan's kinds in the order it always did and hands them
    /// over at the three epochs the table holds now, under
    /// [`IntentSource::Machine`]. The answer is not read: what the machine
    /// learns of a refusal it learns at its next tick, as it always has.
    fn order(&mut self, id: ObjId, plan: &ReflexPlan, t_ms: i64, counters: &mut ReflexCounters) {
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
        // The three epochs read once, before the loop: nothing the loop applies
        // raises an epoch (only `apply_control` does), so these are the epochs
        // each intent read for itself before this cut.
        let epochs = Channel::ALL.map(|channel| (channel, self.epochs.current(id, channel)));
        let epoch_of = |channel: Channel| {
            epochs
                .iter()
                .find(|(on, _)| *on == channel)
                .map_or(0, |(_, epoch)| *epoch)
        };
        let _ = self.apply_machine_intents(
            id,
            &orders,
            t_ms,
            &epoch_of,
            IntentSource::Machine,
            counters,
        );
    }

    /// **Put a machine's intents through the pilot's own door** (17b cut M1a.5;
    /// doc 17 §4, D-UM-12) — `Arena::order`'s body, factored out so the reflex
    /// controller, the motor and a mind's landing are one function.
    ///
    /// The kinds go in emission order, `seq` ascending from `1`, each an
    /// [`Intent`] on the channel that carries it at `epoch_of(channel)`
    /// (`counters.intents` counted per intent), through [`apply_intent`] under
    /// `source`. One answer per kind, in order:
    ///
    /// * refused at apply — [`Landed::Refused`] with `apply_intent`'s word;
    /// * a continuous kind applied — [`Landed::Applied`]; a
    ///   [`MotorGoal::Cell`] that applied runs the motor once here
    ///   (`Arena::motor_step` at `t_ms`), the one follow at landing (D-UM-10);
    /// * a discrete kind queued — popped straight back out of `pending` and
    ///   resolved at `t_ms`: `Startup` by [`resolve_startup`], `Lock` by
    ///   `Arena::resolve_lock`, `Fire` by `Arena::resolve_fire` (`shots`
    ///   counted on `Ok`), and — new in this cut — `Emcon` by
    ///   [`resolve_emcon`]. **Each resolver's `Err` is the answer**
    ///   ([`Landed::Refused`]) and its `Ok` is [`Landed::Applied`]; until this
    ///   cut `Arena::order` dropped them with `let _`, and since it still does
    ///   not read the vector no byte of the reflex controller's moved (the M1a.1
    ///   pins). `Order`, `ReleaseOrder`, `Place` and `Goal` queue nothing.
    pub(crate) fn apply_machine_intents(
        &mut self,
        unit: ObjId,
        kinds: &[IntentKind],
        t_ms: i64,
        epoch_of: &dyn Fn(Channel) -> u64,
        source: IntentSource,
        counters: &mut ReflexCounters,
    ) -> Vec<Landed> {
        let mut landed: Vec<Landed> = Vec::with_capacity(kinds.len());
        let mut seq = 0_u64;
        for kind in kinds {
            let channel = kind.channel_required();
            seq = seq.saturating_add(1);
            let intent = Intent {
                unit,
                channel,
                epoch: epoch_of(channel),
                kind: kind.clone(),
                seq,
            };
            counters.intents = counters.intents.saturating_add(1);
            // **16b cut M.1** (`T-82`): the machine's own order, which the
            // per-seat rate never counts — see `IntentSource`; a mind's
            // (17b cut M1a.5, D-UM-14) neither.
            match apply_intent(self, &intent, source) {
                IntentOutcome::Refused(refusal) => {
                    landed.push(Landed::Refused(refusal));
                    continue;
                }
                IntentOutcome::AppliedContinuous => {
                    // **17b cut M1a.4** (D-UM-10): a goal takes effect at the
                    // landing millisecond, as phase 2's does for a seat's.
                    if matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })) {
                        let _ = self.motor_step(unit, t_ms, counters);
                    }
                    landed.push(Landed::Applied);
                    continue;
                }
                IntentOutcome::Queued => {}
            }
            // `apply_intent` pushed exactly this order to the back of the unit's
            // queue. It comes straight back out: leaving it there would hand it to
            // phase 4, at the pulse's end, which is the arena millisecond this
            // module exists to avoid.
            if let Some(queue) = self.pending.get_mut(&unit) {
                queue.pop_back();
                if queue.is_empty() {
                    self.pending.remove(&unit);
                }
            }
            let resolved = match &intent.kind {
                IntentKind::Startup => resolve_startup(self, unit, t_ms),
                IntentKind::Lock { contact } => self.resolve_lock(unit, *contact, t_ms),
                IntentKind::Fire { weapon } => self.resolve_fire(unit, weapon, t_ms).map(|_| {
                    counters.shots = counters.shots.saturating_add(1);
                }),
                // **17b cut M1a.5** (D-UM-12): the arm `Arena::order` never
                // had. The reflex controller still never switches its pod (16b
                // cut O.2's residual: a machine keeps the switch where it found
                // it), so it never reaches here; a mind's `emcon` does.
                IntentKind::Emcon { on } => resolve_emcon(self, unit, *on, t_ms),
                IntentKind::Heading { .. }
                | IntentKind::Speed { .. }
                | IntentKind::Order(_)
                | IntentKind::ReleaseOrder { .. }
                // 16b cut I.3: a staff act; a machine never issues one, and a
                // mind's is refused `Channel` at apply.
                | IntentKind::Place { .. }
                // 17b cut M1a.4: continuous, so never queued.
                | IntentKind::Goal(_) => Ok(()),
            };
            landed.push(match resolved {
                Ok(()) => Landed::Applied,
                Err(refusal) => Landed::Refused(refusal),
            });
        }
        landed
    }
}

/// **What became of one machine intent** (17b cut M1a.5; doc 17 §4, D-UM-12):
/// `Arena::apply_machine_intents`' answer per kind, in emission order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landed {
    /// It took effect — applied, or queued and resolved.
    Applied,
    /// It was refused, at apply or by its resolver, for this reason. (The
    /// arena's order refusal — this file's own `Refusal` is the motor's
    /// remembered stop, re-exported.)
    Refused(crate::arena::intents::Refusal),
}

/// **The angle between two bearings**, millidegrees, the short way round (16b
/// cut Z.2): `0` to `180,000`, modulo a full turn, so a screen facing 350° sees
/// a plot at 10° as 20° off its facing and not 340°.
///
/// `pub(crate)` since 17b cut M1a.6: the stock mind's screen reads the same
/// arc, in whole degrees.
pub(crate) fn arc_between(a_mdeg: u32, b_mdeg: u32) -> u32 {
    let apart = (a_mdeg % 360_000).abs_diff(b_mdeg % 360_000);
    apart.min(360_000 - apart)
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

impl Arena {
    /// **The squads under a live order, grouped by who gave it** (16b cut Y.2;
    /// doc 16 §2 W8's *orders per commander bounded*): every machine whose
    /// standing order has not expired at arena millisecond `now_ms`, its order's
    /// squad filed under the order's `issuer`.
    ///
    /// An order with no issuer — the LLM seam's — is filed under **the squad
    /// commander of the unit it steers** ([`Arena::squad_of`]), so the machines'
    /// own orders count against the commander that asked for them; a unit in no
    /// squad files it under `None`.
    ///
    /// A pure read for the scale harness's W8 assertion: nothing in the arena
    /// or the world decides anything on it, and nothing but the answer
    /// allocates. It is an **arena-time** figure (`accepted_ms + ttl_ms`), so it
    /// can lead the world's own span — which is counted on the world's watch in
    /// world time ([`crate::arena_bridge::ArenaRegistry::live_orders_of`]) — by
    /// the arena's lag.
    #[must_use]
    pub fn live_orders_by_issuer(
        &self,
        now_ms: i64,
    ) -> BTreeMap<Option<ObjId>, std::collections::BTreeSet<crate::deliberation::SquadId>> {
        let mut grouped: BTreeMap<
            Option<ObjId>,
            std::collections::BTreeSet<crate::deliberation::SquadId>,
        > = BTreeMap::new();
        for (unit, row) in self.reflex.rows() {
            let Some(standing) = row.order.as_ref() else {
                continue;
            };
            if standing.expired_at(now_ms) {
                continue;
            }
            let issuer = standing
                .order
                .issuer
                .or_else(|| self.squad_of(*unit).map(|roster| roster.commander));
            grouped
                .entry(issuer)
                .or_default()
                .insert(standing.order.squad);
        }
        grouped
    }
}

/// The per-unit rows, for [`crate::arena::hash::state_hash`] — the one reader
/// outside this module that needs the machines in order and nothing else.
impl ReflexMemory {
    /// Every machine's state, keyed by unit, in unit-id order.
    pub(crate) fn rows(&self) -> impl Iterator<Item = (&ObjId, &ReflexUnit)> {
        self.units.iter()
    }

    /// **One machine's row, to write** — test builds only (16b cut Z.2's
    /// review): an in-source test that must hand a machine a goal, a route and
    /// a target before proving something drops them has no other door, and no
    /// production reader may write a row from outside `decide`.
    #[cfg(test)]
    pub(crate) fn unit_mut(&mut self, unit: ObjId) -> Option<&mut ReflexUnit> {
        self.units.get_mut(&unit)
    }
}

/// Everything a unit-level test of the controller needs to reach.
#[cfg(test)]
mod tests {
    use super::*;
    // 17b cut M1a.1: the hunt's fan moved to the motor with its picker.
    use crate::arena::motor::fan_rows;
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
            issuer: None,
            by: crate::deliberation::order::CharacterRef::Persona {
                squad: crate::deliberation::squad::SquadId {
                    team: crate::arena::state::Team::Red,
                    index: 1,
                },
            },
            spot: None,
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

    /// **17b cut M1a.1**: through [`Arena::adopt_reflex`], since the goal is
    /// the hull's motor row's — the same three claims, on the two rows.
    #[test]
    fn adopting_a_unit_twice_does_not_restart_its_machine() {
        let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        arena.adopt_reflex(ObjId(4), ReflexState::Startup);
        if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
            row.state = ReflexState::Patrol;
        }
        if let Some(motor) = arena.motors.row_mut(ObjId(4)) {
            motor.goal = Some((3, 3));
        }
        arena.adopt_reflex(ObjId(4), ReflexState::Patrol);
        assert_eq!(
            arena.motors.row(ObjId(4)).and_then(|motor| motor.goal),
            Some((3, 3)),
            "a second adopt into the state it is already in keeps the goal"
        );
        arena.adopt_reflex(ObjId(4), ReflexState::Withdraw);
        assert_eq!(
            arena.reflex.unit(ObjId(4)).map(|row| row.state),
            Some(ReflexState::Withdraw)
        );
        assert_eq!(
            arena.motors.row(ObjId(4)).and_then(|motor| motor.goal),
            None
        );
        arena.release_reflex(ObjId(4));
        assert!(arena.reflex.unit(ObjId(4)).is_none());
        assert!(
            arena.motors.row(ObjId(4)).is_none(),
            "the motor row goes too"
        );
    }

    /// **16b cut Q.7**: each of the four new row fields is authoritative state,
    /// so each of them moves [`crate::arena::hash::state_hash`].
    #[test]
    fn each_new_row_field_moves_the_state_hash() {
        let build = || {
            let mut arena = arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
            arena.adopt_reflex(ObjId(4), ReflexState::Withdraw);
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
            // 17b cut M1a.1: the avoided cell is the motor row's; it is folded
            // in the machine's own place in the row (`hash.rs` item 3).
            if let Some(motor) = arena.motors.row_mut(ObjId(4)) {
                match edit {
                    0 => motor.avoid.push(((3, 4), 1_000)),
                    1 => motor.avoid.push(((3, 4), 2_000)),
                    _ => {}
                }
            }
            if let Some(row) = arena.reflex.units.get_mut(&ObjId(4)) {
                match edit {
                    0 | 1 => {}
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

    /// **17b cut M1a.1 (`hash.rs` item 3b): a motor row of a hull the reflex
    /// controller is not flying is folded, field by field, and a table that
    /// holds none adds no byte.**
    ///
    /// A pilot's tank with no machine is given a motor row — the shape 17b cut
    /// M1a.4's `DRIVE` lays down — and each of the row's folded fields, set one
    /// at a time, moves the hash to a state no earlier probe reached; taking the
    /// row away is the first state again. And a reflex row whose hull has no
    /// motor row (the table emptied under it) hashes exactly as one whose row
    /// is fresh: item 3 folds a missing row as `MotorRow::default()`, which is
    /// what the fields folded while they were the machine's. That every match
    /// that exists hashes as it did is the two HEAD pins' to prove
    /// (`tests/reflex.rs`, `tests/motor.rs`).
    #[test]
    fn m1a1_a_motor_row_of_a_unit_with_no_reflex_row_moves_the_hash_and_an_empty_table_adds_none() {
        use crate::arena::motion::BlockedCause;
        use crate::arena::motor::MotorRow;
        let build = || arena_of(vec![tank(ObjId(4), Team::Red, pose(1_500, 1_500))]);
        let bare = build();
        assert!(bare.reflex.unit(ObjId(4)).is_none(), "a pilot's tank");
        assert_eq!(bare.motors.rows().count(), 0, "an empty table");
        let base = crate::arena::hash::state_hash(&bare);

        let refusal = Refusal {
            at: (1, 1),
            cell: (2, 1),
            cause: BlockedCause::Impassable,
            goal: (6, 4),
            until_ms: 31_000,
        };
        let mut seen: Vec<u64> = vec![base];
        for edit in 0_u8..=8 {
            let mut arena = build();
            let motor = arena.motors.ensure(ObjId(4));
            match edit {
                0 => {} // the row itself, every field at its default
                1 => motor.goal = Some((3, 4)),
                2 => motor.goal = Some((4, 3)),
                3 => motor.waypoint = Some(2),
                4 => motor.route = vec![(2, 2)],
                5 => motor.avoid = vec![((3, 4), 1_000)],
                6 => motor.hold_retry_ms = Some(9_000),
                7 => motor.searched_from = Some((1, 1)),
                _ => motor.refused = Some(refusal),
            }
            let hash = crate::arena::hash::state_hash(&arena);
            assert!(
                !seen.contains(&hash),
                "probe {edit} of a machine-less motor row folded the bytes of an earlier state"
            );
            seen.push(hash);
            arena.motors.release(ObjId(4));
            assert_eq!(
                crate::arena::hash::state_hash(&arena),
                base,
                "probe {edit}: the row's release left a byte behind"
            );
        }
        // `from_waypoint` is deliberately not folded, as it was not on the
        // machine (item 3): a row differing only there is one state.
        let mut arena = build();
        arena.motors.ensure(ObjId(4)).from_waypoint = Some(1);
        let mut other = build();
        let _ = other.motors.ensure(ObjId(4));
        assert_eq!(
            crate::arena::hash::state_hash(&arena),
            crate::arena::hash::state_hash(&other)
        );

        // A machine whose motor row is missing hashes as one whose row is fresh.
        let mut fresh = build();
        fresh.adopt_reflex(ObjId(4), ReflexState::Patrol);
        assert_eq!(fresh.motors.row(ObjId(4)), Some(&MotorRow::default()));
        let mut emptied = build();
        emptied.adopt_reflex(ObjId(4), ReflexState::Patrol);
        emptied.motors.release(ObjId(4));
        assert_eq!(emptied.motors.rows().count(), 0);
        assert_eq!(
            crate::arena::hash::state_hash(&fresh),
            crate::arena::hash::state_hash(&emptied),
            "a fresh motor row and no motor row fold the same bytes under a machine"
        );
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

    /// **16b cut Z.2's review**: a screen's arc is measured the short way round.
    /// Every screen the replays and the lance lane fly faces 090° or 270°, where
    /// a plain difference of bearings gives the same answers; a screen facing
    /// 000° or 350° is where the wrap decides whether a tank at 10° is met.
    #[test]
    fn z2_the_arc_between_two_bearings_is_the_short_way_round() {
        for (a, b, apart) in [
            (350_000_u32, 10_000_u32, 20_000_u32),
            (10_000, 350_000, 20_000),
            (0, 359_000, 1_000),
            (359_999, 0, 1),
            (90_000, 270_000, 180_000),
            (90_000, 30_000, 60_000),
            (0, 0, 0),
            // A facing written a full turn over is the same facing.
            (360_000, 0, 0),
            (720_000 + 5_000, 355_000, 10_000),
        ] {
            assert_eq!(arc_between(a, b), apart, "{a} and {b}");
        }
        // The half arc admits a tank at 10° to a screen facing north, and a tank
        // at 61° off it is outside.
        assert!(arc_between(10_000, 0) <= SCREEN_HALF_ARC_MDEG);
        assert!(arc_between(299_000, 0) > SCREEN_HALF_ARC_MDEG);
    }

    // -----------------------------------------------------------------------
    // 16b cut AI.1 (16c `B-84`)
    // -----------------------------------------------------------------------

    /// **16b cut AI.1, the programme lead's ruling on `shipped_scale` e3: the
    /// hunters of one side are handed cells clear of each other's, wherever
    /// they stand, and the same ones at every cadence.**
    ///
    /// On the test window (Blue's zone is columns 6 and 7, every row), three Red
    /// tanks on row 3 all clamp to Blue's corner (6, 3). With no hunt held by
    /// the side each is handed that corner, E.4's cell; handed one after
    /// another, each against the cells handed before it, they get rows 3, 6 and
    /// 0 — and from one cell exactly, since nothing here reads a pose but the
    /// unit's own. A unit whose own row is already [`HUNT_FAN_ROWS`] from the
    /// held cell keeps its own row (the cross-row case a fan by rank alone got
    /// wrong); one two rows from it is fanned; a zone with no row left clear
    /// answers E.4's cell, the named residual. [`fan_rows`] is pinned row for
    /// row from every row of the window.
    ///
    /// Then three of them hunt for real from their opening cells — two on row
    /// 3 and one on row 6, the lowest id on row 6, the layout in which a fan by
    /// side ordinal hands the lowest two ids one cell — no hostile anywhere,
    /// stepped at 5,000 / 1,000 / 500 ms: one hash sequence, one set of hunt
    /// cells at every shared boundary, and the three cells pairwise clear.
    #[test]
    fn ai1_the_hunters_of_one_side_are_handed_clear_cells_at_three_cadences() {
        let red = |id: u32, x: u16, y: u16| {
            tank(
                ObjId(id),
                Team::Red,
                pose(i64::from(x) * 1_000 + 500, i64::from(y) * 1_000 + 500),
            )
        };
        let apart = |a: (u16, u16), b: (u16, u16)| a.0.abs_diff(b.0).max(a.1.abs_diff(b.1));
        // (a) The answer itself.
        let mut arena = arena_of(vec![
            tank(ObjId(3), Team::Blue, pose(7_500, 7_500)),
            red(4, 2, 3),
            red(5, 3, 3),
            red(6, 4, 3),
        ]);
        assert_eq!(
            [4, 5, 6].map(|id| arena.far_entry_cell(ObjId(id), &[])),
            [Some((6, 3)); 3],
            "no hunt held: every machine is handed E.4's clamped corner"
        );
        let first = arena.far_entry_cell(ObjId(4), &[]);
        assert_eq!(first, Some((6, 3)));
        let second = arena.far_entry_cell(ObjId(5), &[(6, 3)]);
        assert_eq!(second, Some((6, 6)), "the fan's first row");
        let third = arena.far_entry_cell(ObjId(6), &[(6, 3), (6, 6)]);
        assert_eq!(third, Some((6, 0)), "the fan's second row");
        // From one cell exactly: the held cells, not the poses, keep them apart.
        let one = arena.unit(ObjId(4)).map(|unit| unit.pose.clone());
        for id in [5, 6] {
            if let (Some(state), Some(at)) = (arena.unit_mut(ObjId(id)), one.clone()) {
                state.pose = at;
            }
        }
        assert_eq!(
            [
                arena.far_entry_cell(ObjId(4), &[]),
                arena.far_entry_cell(ObjId(5), &[(6, 3)]),
                arena.far_entry_cell(ObjId(6), &[(6, 3), (6, 6)]),
            ],
            [Some((6, 3)), Some((6, 6)), Some((6, 0))],
            "three machines in one cell, three hunt cells"
        );
        // Across rows: a held cell three rows off leaves the unit its own row;
        // one two rows off fans it; a zone with every fanned row's cell held
        // answers E.4's cell after all.
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 6)]),
            Some((6, 3)),
            "three rows apart is clear"
        );
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 5)]),
            Some((6, 0)),
            "two rows apart is not: row 6 is one from it, row 0 is clear"
        );
        assert_eq!(
            arena.far_entry_cell(ObjId(4), &[(6, 0), (6, 3), (6, 6)]),
            Some((6, 3)),
            "the fan ran out, so E.4's cell"
        );
        // Another side's hunts are no business of Blue's: `taken` is the
        // caller's, and Blue's own is empty.
        assert_eq!(arena.far_entry_cell(ObjId(3), &[]), Some((1, 7)));
        // The fan's rows, from every row of an eight-row zone: the clamped row
        // first, then alternately above and below in steps of three, rows off
        // the zone skipped and nothing repeated.
        let expected: [&[u16]; 8] = [
            &[0, 3, 6],
            &[1, 4, 7],
            &[2, 5],
            &[3, 6, 0],
            &[4, 7, 1],
            &[5, 2],
            &[6, 3, 0],
            &[7, 4, 1],
        ];
        for (cy, want) in (0..=7_u16).zip(expected) {
            let rows: Vec<u16> = fan_rows(cy, 0, 7).collect();
            assert_eq!(rows, want, "the fan from row {cy}");
            for (i, a) in rows.iter().enumerate() {
                for b in rows.iter().skip(i + 1) {
                    assert!(
                        a.abs_diff(*b) >= HUNT_FAN_ROWS,
                        "rows {a} and {b} from row {cy}"
                    );
                }
            }
        }
        assert_eq!(fan_rows(4, 4, 4).collect::<Vec<_>>(), vec![4], "one row");

        // (b) Three machines hunting for real. Their hunt clocks are set run
        // out, so each crosses at its first tick, from the cell it opened in;
        // and they are slowed to 200 cm/s, so the crossing is still under way at
        // the boundaries all three cadences share.
        let hunters = || {
            let mut arena = arena_of(vec![red(4, 2, 6), red(5, 2, 3), red(6, 3, 3)]);
            let clock = i64::from(arena.reflex.cfg.hunt_after_ms);
            for id in [4, 5, 6] {
                arena.set_controller(ObjId(id), Controller::Reflex, ReflexState::Patrol);
                if let Some(state) = arena.unit_mut(ObjId(id)) {
                    state.power = crate::arena::state::Power::Online;
                    state.mobility.max_speed_cms = 200;
                }
                if let Some(row) = arena.reflex.units.get_mut(&ObjId(id)) {
                    row.unheld_since_ms = Some(-clock);
                }
            }
            arena
                .clock
                .pin_tempo_for_tests(crate::arena::clock::Tempo::Quiet);
            arena
        };
        // Per cadence: the per-substep hashes, and the three machines' hunt cells
        // and cells at every boundary all three cadences share (each 5,000 ms).
        type Seen = (i64, [Option<(u16, u16)>; 3], [Option<(u16, u16)>; 3]);
        type Run = (Vec<u64>, Vec<Seen>);
        let mut runs: Vec<Run> = Vec::new();
        for cadence in [5_000_i64, 1_000, 500] {
            let mut arena = hunters();
            let mut hashes: Vec<u64> = Vec::new();
            let mut seen: Vec<Seen> = Vec::new();
            let mut wall = 0_i64;
            loop {
                arena.step(wall, &[], &[]);
                hashes.extend(arena.substep_hashes.iter().copied());
                if arena.now_ms % 5_000 == 0 {
                    let hunt = |id: u32| arena.reflex.unit(ObjId(id)).and_then(|row| row.hunt);
                    let cell = |id: u32| {
                        arena
                            .unit(ObjId(id))
                            .and_then(|unit| arena.definition.terrain.cell_of(&unit.pose))
                    };
                    seen.push((
                        arena.now_ms,
                        [hunt(4), hunt(5), hunt(6)],
                        [cell(4), cell(5), cell(6)],
                    ));
                }
                if arena.now_ms >= 10_000 {
                    break;
                }
                wall += cadence;
            }
            runs.push((hashes, seen));
        }
        let coarse = runs.first().cloned().unwrap_or_default();
        println!("ai1_fan: {:?}", coarse.1);
        let crossing = coarse.1.iter().find(|(at, _, _)| *at == 5_000).copied();
        let Some((_, [Some(a), Some(b), Some(c)], _)) = crossing else {
            panic!(
                "all three machines were crossing at 5,000 ms: {:?}",
                coarse.1
            );
        };
        let mut cells = [a, b, c];
        cells.sort_unstable();
        assert_eq!(
            cells,
            [(6, 0), (6, 3), (6, 6)],
            "rows 6 and 3 are each some machine's own, and the second machine of \
             row 3 is fanned to the one row left clear"
        );
        for (x, y) in [(a, b), (a, c), (b, c)] {
            assert!(
                apart(x, y) >= HUNT_FAN_ROWS,
                "two hunt cells overlap: {x:?} {y:?}"
            );
        }
        for run in runs.iter().skip(1) {
            assert_eq!(run.0, coarse.0, "the hash sequence diverged");
            assert_eq!(run.1, coarse.1, "the hunt cells diverged");
        }
    }
}
