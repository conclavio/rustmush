//! **The pulse** (16b cut A.2; doc 16 §3.1 "Pulse phases", "Rate independence",
//! "Clock and lag" and "Determinism").
//!
//! One function, [`Arena::step`], and the five phases of doc 16 §3.1 in order,
//! *each reading the state the previous phase committed*. It is a pure function of
//! the arena's state and its two argument slices: no clock of its own (the wall
//! reading is an argument), no task, no I/O. Cut T's `arena_task` is the thin
//! wrapper that owns the timer, the bounded inbox, the bounded egress and the
//! unwind boundary.
//!
//! # The phases, and each one's snapshot rule
//!
//! 1. **Authority.** Every [`Control`] is applied to the epoch table, which purges
//!    queued orders below the new epoch — so a pilot who types `FIRE` then `EXIT`
//!    in one burst never fires, because the burst's `Control` is applied *before*
//!    the burst's `Intent`s. A `Stop` the world sent is honoured here and nowhere
//!    else: it never lands in the middle of a substep.
//! 2. **Orders.** Every [`Intent`] is authorised *at apply* against the table
//!    phase 1 just committed. Continuous orders set a desired value, latest wins;
//!    discrete ones queue for phase 4.
//! 3. **Time.** The wall reading becomes a clamped `dt`, and `dt` becomes a
//!    sequence of substeps. Each substep: (a) motion integrates every unit in
//!    unit-id order **from the substep-start snapshot**, with occupancy read from
//!    the index as the earlier movers left it; (b) the index is rebuilt; (c) arena
//!    time advances by the substep and every due event at or before it is applied
//!    in `(t_ms, seq)` order. The substep's ledger events are appended last,
//!    against the hash of the state at the substep's **end**.
//! 4. **Actions.** Each unit's queue is drained in unit-id order and resolved
//!    against the world phase 3 left. Nothing is re-queued: an action is resolved
//!    or refused in the pulse it was drained in.
//! 5. **The pulse's own record.** The tempo is evaluated, lag is journalled, the
//!    bounds are checked, the ledger's closed chunks are taken and a frame is
//!    built for every pilot whose view changed.
//!
//! # Why the hash is computed once per substep
//!
//! [`crate::arena::hash::state_hash`] folds the ledger's event sequence, so the
//! hash moves with every append. Every event of a substep therefore carries **one**
//! hash — the state at the substep's end, before any of that substep's events was
//! appended — which is what makes the value a function of the state rather than of
//! the order the events happened to be written in. The sequence of those hashes is
//! kept on [`crate::arena::state::Arena::substep_hashes`] for the pulse just run,
//! which is how the cross-tempo property test compares three cadences substep by
//! substep instead of only at the end.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use mush_core::ObjId;
use mush_sim::data::Pose;
use mush_tactics::{CellAddr, CellId, FacetId, ScalarCell, SectionId, SectionState};
use smol_str::SmolStr;

use crate::arena::clock::{Tempo, TempoSignals, substep_ms, substeps};
use crate::arena::conditions::ConditionTrigger;
use crate::arena::hash::state_hash;
use crate::arena::intents::{
    Control, Intent, IntentKind, IntentOutcome, IntentSource, MotorGoal, Refusal, apply_control,
    apply_intent, drain_pending, resolve_emcon, resolve_startup,
};
use crate::arena::ledger::{EventKind, LedgerChunk};
use crate::arena::mind::SORTIE_MATCH_END;
use crate::arena::motion::integrate;
use crate::arena::motor::MotorCounters;
use crate::arena::perception::{
    Aspect, FrameCondition, FrameContact, FrameNotice, FrameOdds, FrameOrder, Named, Notice,
    NoticeKind, PerceptionCounters, RefusalDetail,
};
use crate::arena::resolution::facet_band_seen;
use crate::arena::state::{
    ActionHash, Arena, Classification, ContactId, Controller, DueEvent, DueKind, EndReason,
    MatchEnd, Power, SPENT_NOTICE_MS, StopReason, Team,
};
use crate::arena::weapons::FrameWeapon;
use crate::definitions::unit::{armor_cell, internals_cell};

mod frames;
mod movement;
mod sensors;
#[cfg(test)]
mod tests;
mod weapons;

pub use self::frames::{
    Frame, FrameFacet, FrameHeat, FrameMind, FrameMotor, FrameSection, FrameSlots, STOCK_ODDS_ROWS,
};
#[cfg(test)]
use self::frames::{frame_sections_merged, rank_facet, rank_section, take_at};
use self::movement::{PLACE_STOPPED, floor_to};

/// **What one pulse produced** (doc 16 §3.1: `StepOutput { ledger_chunks, frames,
/// tempo, next_due_ms }`, plus what cut T's task needs to schedule and to stop).
#[derive(Debug)]
pub struct StepOutput {
    /// Ledger chunks that closed and have not been handed over before. The world
    /// half appends them to `log:match.<id>` on its own writer lane and
    /// acknowledges them with `Ledger::mark_flushed`.
    pub ledger_chunks: Vec<LedgerChunk>,
    /// One frame per **pilot** whose view changed this pulse, in pilot-id order.
    pub frames: Vec<(ObjId, Frame)>,
    /// The tempo the arena is now in — the period cut T's timer uses.
    pub tempo: Tempo,
    /// When the task should step again, in the **wall** milliseconds
    /// `wall_now_ms` was given in: the earlier of the next multiple of the period
    /// and the first substep boundary at or after the next due event, both in
    /// absolute arena time (16b cut U.1; `Arena::next_due_ms` has the rule and
    /// why), so a production wake is on the substep lattice and never at a due
    /// event's own millisecond.
    pub next_due_ms: i64,
    /// The pulse absorbed more wall time than the clamp allows, or the egress is
    /// backing up.
    pub lagged: bool,
    /// Set once, on the pulse the match ends.
    pub ended: Option<MatchEnd>,
    /// How many substeps this pulse ran, the phase-4 drain included; the opening
    /// pulse runs none, because it only establishes the wall reference.
    ///
    /// **16b cut M.2** (16c `T-10`): it is counted as the substeps run, so a pulse
    /// whose second of four substeps ended the match reports `2` — it reported
    /// the whole partition — and a pulse whose phase 4 scheduled a zero-flight
    /// event reports one more than its partition (`Arena::step`'s phase-4 drain).
    pub substeps: u32,
    /// **What perception cost this pulse** (16b cut S.1; doc 16 §2 W4 and W6). The
    /// harness asserts `harness.max_traces_per_pulse` and
    /// `harness.max_share_ops_per_pulse` against it.
    pub perception: PerceptionCounters,
    /// **What the machines cost this pulse** (16b cut N.1; doc 16 §1 A2: *path
    /// queries per pulse under their bound*; 17b cut M4.5: was `reflex`). The
    /// harness asserts `harness.max_path_queries_per_pulse` against
    /// [`MotorCounters::path_queries`] and `max_path_cells_per_pulse` against
    /// [`MotorCounters::path_cells`].
    pub motor: MotorCounters,
    /// **The squads with something new to decide about** (16b cut N.2; doc 16 §6
    /// "Deliberation"), already floored by `[tacsim.arena].deliberate_min_ms` in
    /// arena time.
    ///
    /// Empty on every pulse of a match with no squads, which is every fixture and
    /// every match of pilots. The world half turns each into one request through
    /// its own LLM re-entry seam and the reply comes back as an ordinary
    /// [`crate::arena::intents::IntentKind::Order`]; the arena waits for neither
    /// and is told nothing if the world drops it.
    pub deliberation: Vec<crate::arena::command::SquadSignal>,
    /// **The staff picture of the field** (16b cut I.4; 16c N10), built in
    /// phase 5 while [`crate::arena::state::Arena::watched`] is set, **at most
    /// once per pulse and at most once an arena second** — and on the pulse the
    /// match ended, so a watcher sees the end. `None` otherwise, on the ended
    /// early return and on a stopped pulse.
    ///
    /// A type of its own ([`crate::arena::observer::Observer`]) and never part
    /// of [`StepOutput::frames`]: it names every unit, and a pilot's frame names
    /// only what their platform holds (doc 16 §1).
    pub observer: Option<crate::arena::observer::Observer>,
    /// **The thinks this pulse collected and has not written** (17b cut
    /// M4.1; doc 17 §6 "Execution policy", D-UM-22): those of its last
    /// substep, frozen ([`crate::arena::mind::CollectedThink`]) and stamped
    /// with their substep's hash — clones of the arena's own list. The sim
    /// task hands them to the process's mind pool after `publish` and
    /// delivers the answers back (`Arena::deliver_answers`) before its next
    /// step and before it encodes a snapshot; a caller with no pool runs them
    /// itself (`crate::arena::mind::run_batch_inline`) or leaves them, and
    /// the next `Arena::step` writes them inline at its head. Empty on a
    /// pulse that collected none, on the ended early return and on a stopped
    /// pulse; always empty from [`Arena::step_settled`].
    pub thinks: Vec<crate::arena::mind::CollectedThink>,
}

// ---------------------------------------------------------------------------
// The two hidden fault-injection hooks (16b cut T.2)
// ---------------------------------------------------------------------------
//
// Cut T's task owes two properties the library cannot be asked to demonstrate on
// its own: *a panic inside `step` ends the match `Void` and nothing the faulted
// pulse produced leaves*, and *a pulse past `tempo.stall_ms` is voided by the
// watchdog* (doc 16 §3.1 "Failure"). Both need a `step` that misbehaves on
// demand, and neither can be produced by any legal input — which is exactly why
// they are hooks and not test fixtures.
//
// They are armed **by seed**, not by a flag on the arena: [`Arena`] is
// `state.rs`' and cut T is not allowed to add a field to it, and a plain global
// flag would be armed by one async test and consumed by another running
// beside it in the same binary. The arming method takes `&mut self` and reads
// `self.seed`, so a test can only ever arm the arena it is holding, and two
// tests with two seeds cannot collide. A seed of `0` disarms (and no test uses
// it).
//
// `std::panic::resume_unwind` rather than `panic!`: this crate's root denies
// `clippy::panic` (which also refuses `panic_any`) and
// `panic_budget/tacsim.txt` counts the `panic!` spelling, and both exist to stop
// *accidental* panic surface. A `#[doc(hidden)]` hook whose whole job is to
// unwind, reachable only from the test that armed it by seed, is not that — and
// silencing the lint would move the pinned restriction-lint census instead,
// which is strictly worse than one named call in one place. `resume_unwind`
// raises exactly the unwind `catch_unwind` catches, with exactly the payload
// given, and skips the panic hook — so a test that expects the fault does not
// print a backtrace for it either.

/// The seed of the arena whose next [`Arena::step`] must panic; `0` for none.
static PANIC_SEED: AtomicU64 = AtomicU64::new(0);
/// The seed of the arena whose next [`Arena::step`] must stall; `0` for none.
static STALL_SEED: AtomicU64 = AtomicU64::new(0);
/// How long that stall lasts, milliseconds.
static STALL_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
enum PulseStage {
    Integrate,
    Actions,
    Drain,
    Finish,
}
struct PulseWork {
    wall_now_ms: i64,
    perception: PerceptionCounters,
    reflex: MotorCounters,
    shots_before: u64,
    fire_intent_queued: bool,
    dt_ms: u32,
    clock_lagged: bool,
    floored: bool,
    pieces: std::vec::IntoIter<u32>,
    ran: u32,
    ledger_full: bool,
    stage: PulseStage,
}
enum PulseProgress {
    Running(PulseWork),
    Done(StepOutput),
}

/// One scheduler turn. Zero-time phases still consume a turn, preventing a
/// stream of control/ack/pulse barriers from monopolizing a bulk worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PulseSlice {
    pub at_ms: i64,
    pub simulated_ms: u32,
    pub done: bool,
}

/// Private arena custody during a pulse. There is deliberately no arena/view,
/// snapshot, mutation, or source getter while a continuation is active. Dropping
/// this owner cancels private work; live `step` drives the identical phases.
pub struct OwnedPulse {
    arena: Arena,
    progress: Option<PulseProgress>,
}
impl OwnedPulse {
    pub fn new(
        mut arena: Arena,
        wall_now_ms: i64,
        controls: &[Control],
        intents: &[Intent],
    ) -> Self {
        let progress = arena.begin_pulse(wall_now_ms, controls, intents);
        Self {
            arena,
            progress: Some(progress),
        }
    }
    pub fn next_simulation_ms(&self) -> u32 {
        match self.progress.as_ref() {
            Some(PulseProgress::Running(work)) if matches!(work.stage, PulseStage::Integrate) => {
                work.pieces.as_slice().first().copied().unwrap_or(0)
            }
            _ => 0,
        }
    }
    pub fn advance(&mut self) -> PulseSlice {
        let before = self.arena.now_ms;
        self.progress = self.progress.take().map(|progress| match progress {
            PulseProgress::Running(work) => self.arena.continue_pulse(work),
            done => done,
        });
        PulseSlice {
            at_ms: self.arena.now_ms,
            simulated_ms: u32::try_from(self.arena.now_ms.saturating_sub(before))
                .unwrap_or(u32::MAX),
            done: matches!(self.progress, Some(PulseProgress::Done(_))),
        }
    }
    // An unfinished pulse returns ownership intact so the caller can resume it;
    // boxing only that continuation would add an allocation on the yield path.
    #[allow(clippy::result_large_err)]
    pub fn finish(mut self) -> Result<(Arena, StepOutput), Self> {
        match self.progress.take() {
            Some(PulseProgress::Done(output)) => Ok((self.arena, output)),
            progress => {
                self.progress = progress;
                Err(self)
            }
        }
    }
}

impl Arena {
    /// **Test hook:** make the next [`Arena::step`] of *this* arena panic,
    /// once.
    ///
    /// Nothing in the shipped lane calls it. See this module's
    /// fault-injection note for why it is armed by seed.
    #[doc(hidden)]
    pub fn panic_on_next_step_for_tests(&mut self) {
        PANIC_SEED.store(self.seed, Ordering::SeqCst);
    }

    /// **Test hook:** make the next [`Arena::step`] of *this* arena block for
    /// `how_long` before doing its work, once — a pulse the task's watchdog
    /// must void.
    ///
    /// It blocks the calling thread, because that is what a stalled pulse does:
    /// the arena has no preemption, and the watchdog reads the clock *after*
    /// `step` returns.
    #[doc(hidden)]
    pub fn stall_next_step_for_tests(&mut self, how_long: std::time::Duration) {
        STALL_MS.store(
            u64::try_from(how_long.as_millis()).unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        STALL_SEED.store(self.seed, Ordering::SeqCst);
    }

    /// Consume whichever fault this arena's seed is armed with. A no-op — one
    /// relaxed load of each of two atomics — on every arena nobody armed, which
    /// is every arena in the shipped lane.
    fn injected_fault(&self) {
        if self.seed != 0 && PANIC_SEED.load(Ordering::SeqCst) == self.seed {
            PANIC_SEED.store(0, Ordering::SeqCst);
            std::panic::resume_unwind(Box::new(
                "arena step panicked (panic_on_next_step_for_tests)",
            ));
        }
        if self.seed != 0 && STALL_SEED.load(Ordering::SeqCst) == self.seed {
            STALL_SEED.store(0, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(
                STALL_MS.swap(0, Ordering::SeqCst),
            ));
        }
    }

    /// **One pulse** — doc 16 §3.1's five phases, in order.
    ///
    /// `wall_now_ms` is the caller's monotonic wall reading; arena time advances by
    /// the interval between two readings, clamped to `max_catch_up_steps × period`
    /// (doc 16 §3.1 "Clock and lag"). The first call establishes the reference and
    /// integrates nothing, so the opening pulse is identical at every tempo.
    ///
    /// A match that has already ended does nothing at all: the output carries its
    /// [`MatchEnd`] and no chunk, no frame and no substep. Cut T stops stepping at
    /// the first output with `ended`; a task that steps once more anyway cannot
    /// move the world afterwards.
    pub fn step(
        &mut self,
        wall_now_ms: i64,
        controls: &[Control],
        intents: &[Intent],
    ) -> StepOutput {
        let mut pulse = self.begin_pulse(wall_now_ms, controls, intents);
        loop {
            pulse = match pulse {
                PulseProgress::Done(output) => return output,
                PulseProgress::Running(work) => self.continue_pulse(work),
            };
        }
    }

    fn begin_pulse(
        &mut self,
        wall_now_ms: i64,
        controls: &[Control],
        intents: &[Intent],
    ) -> PulseProgress {
        self.injected_fault();
        if let Some(ended) = self.ended {
            return PulseProgress::Done(StepOutput {
                ledger_chunks: Vec::new(),
                frames: Vec::new(),
                tempo: self.clock.tempo(),
                next_due_ms: wall_now_ms,
                lagged: false,
                ended: Some(ended),
                substeps: 0,
                perception: PerceptionCounters::default(),
                motor: MotorCounters::default(),
                deliberation: Vec::new(),
                observer: None,
                thinks: Vec::new(),
            });
        }
        self.substep_hashes.clear();
        let perception = PerceptionCounters::default();
        let mut reflex = MotorCounters::default();
        // **17b cut M4.1**: the thinks the last pulse's last substep
        // collected, when nothing delivered their answers before this step
        // (`Arena::deliver_answers` — the sim task's pool, or
        // `Arena::step_settled`), are written NOW, before phase 1: the one
        // sim-time moment the pooled path writes them at too — after
        // everything the last pulse journalled, before anything of this one.
        let settled = self.settle_inline();
        reflex.mind_faults = reflex.mind_faults.saturating_add(settled.faults);
        reflex.mind_overflows = reflex.mind_overflows.saturating_add(settled.overflows);
        // **16b cut N.1**: what the pulse fired, wherever it fired it. Cut W.1 read
        // this off phase 4's own return, which stopped being the whole answer the
        // moment the reflex controller began accepting shots inside a substep. The
        // shot sequence is the one counter that moves on every accepted shot and on
        // nothing else, so the difference across the pulse is the fact.
        let shots_before = self.next_shot_seq;

        // ---- phase 1: authority, and the world's `Stop` ----
        //
        // Controls first, and every control before any intent: the epoch table
        // this phase commits is what phase 2 authorises against, which is the
        // whole mechanism behind doc 16 §3.1's `FIRE` then `EXIT` burst.
        for control in controls {
            apply_control(self, control);
        }
        if let Some(stop) = self.stop {
            // 17b cut M1b.5: a stopped match takes no card change.
            self.pending_cards.clear();
            return PulseProgress::Done(self.stopped(wall_now_ms, stop, intents));
        }

        // ---- phase 2: orders ----
        //
        // **16b cut D.2** (`B-03`): this loop used to throw every refusal away, so
        // an order to a dead tank, to a tank whose engine was gone, or one past
        // the queue's bound simply vanished — three of six smoke matches left a
        // pilot typing into a wreck and being told nothing. Four of the refusals
        // are now staged (five since 17b cut M5.2's `Refusal::Profile`), and the
        // other five are silent on purpose: see [`Arena::phase_two_tells`].
        let mut fire_intent_queued = false;
        // **16b cut Y.1**: phase 2 runs at one arena timestamp, so every `ordered`
        // line of one pulse belongs to one hash — an `ORDER ALL` over four pairs
        // costs one `state_hash`, not eight. 16b cut M.3's rule (`T-23`) kept: the
        // slot is `Pending`, the first `ordered` line takes it
        // (`Arena::event_hash`), and a pulse with no order takes nothing. **16b
        // cut Z.2**: the `order_released` line is the other thing phase 2
        // journals, and it takes the same slot, so the `ordered` and
        // `order_released` lines of one pulse share one hash. **16b cut I.3**: so
        // does the `place_refused` line of a staff place this phase refuses;
        // nothing else here journals (`stage_refusal` writes no ledger line).
        self.action_hash = ActionHash::Pending;
        // **17b cut M1b.5** (doc 17 §11, §22's M1b): the card changes staff
        // offered since the last pulse land first, at this pulse's start and
        // under this phase's one hash, so the intents of the same pulse — and
        // every think from here on — read the new card.
        self.apply_mind_cards();
        for intent in intents {
            // **16b cut Z.2** (16c Q44): a release is counted where it lands — a
            // unit whose standing order the release took away. A release to a
            // unit under no order, or under another squad's, dropped nothing and
            // counts nothing.
            //
            // **17b cut M1a.5**: a mind's held order counts the same way —
            // and since 17b cut M4.6 deleted the reflex's rows, it is the only
            // held order there is.
            let held_order = |arena: &Arena| {
                arena
                    .minds
                    .get(&intent.unit)
                    .is_some_and(|state| state.order.is_some())
            };
            let releasing =
                matches!(intent.kind, IntentKind::ReleaseOrder { .. }) && held_order(self);
            // **16b cut M.1** (`T-82`): every order in this slice came through the
            // world's seam, so it is a seat's and counts against the rate.
            let outcome = apply_intent(self, intent, IntentSource::Seat);
            if releasing && !held_order(self) {
                reflex.orders_released = reflex.orders_released.saturating_add(1);
            }
            if outcome == IntentOutcome::Queued && matches!(intent.kind, IntentKind::Fire { .. }) {
                fire_intent_queued = true;
            }
            // **17b cut M1a.4** (doc 17 §4 D-UM-10: *when a `Goal` lands, the
            // motor runs one `follow` at the landing millisecond, so a new goal
            // takes effect at N+1 and does not wait for the next motor tick*).
            // Phase 2 runs at the pulse's start, and a world beat lands in the
            // pulse that reaches it, so the step happens at one arena
            // millisecond at every cadence. A hold or a release of the helm
            // drives nothing.
            if outcome == IntentOutcome::AppliedContinuous
                && matches!(intent.kind, IntentKind::Goal(MotorGoal::Cell { .. }))
            {
                let _ = self.motor_step(intent.unit, self.now_ms, &mut reflex);
            }
            // **16b cut I.3** (16c N10: *journalled always, announced never*):
            // a staff `Place` refused here — `unknown`, `epoch` or `channel`, the
            // three gates it meets — still leaves its line, under phase 2's one
            // hash like the `ordered` lines, so every `Place` the arena receives
            // is on the tape by one rule. Nothing is staged:
            // `Arena::phase_two_tells` answers `false` for a place.
            if let (IntentOutcome::Refused(refusal), IntentKind::Place { x, y }) =
                (outcome, &intent.kind)
            {
                let hash = self.event_hash();
                let at = self.now_ms;
                let _ = self.journal(
                    at,
                    EventKind::PlaceRefused {
                        unit: intent.unit,
                        x: *x,
                        y: *y,
                        reason: SmolStr::new_static(refusal.as_str()),
                    },
                    hash,
                );
            }
            if let IntentOutcome::Refused(refusal) = outcome
                && Arena::phase_two_tells(&intent.kind, refusal)
            {
                let weapon = match &intent.kind {
                    IntentKind::Fire { weapon, .. } => Some(weapon.clone()),
                    // 17b cut M1b.1: an arming told `queue_full` or `wreck`
                    // here names its weapon as phase 4's landing refusals do,
                    // so it reads `FIRE cannon refused. …` — the shape every
                    // `FIRE` refusal has.
                    IntentKind::Condition(arming) => arming.weapon().cloned(),
                    _ => None,
                };
                // The word is `IntentKind::as_str`'s and phase 4 uses the same
                // accessor, so the two phases cannot spell one order two ways.
                self.stage_refusal(intent.unit, intent.kind.as_str(), refusal, weapon);
            }
        }
        // **16b cut Y.1**: the phase-2 window closes before time moves.
        self.action_hash = ActionHash::Outside;

        // ---- phase 3: time, in substeps ----
        //
        // **16b cut I.3** (16c N10; `crate::arena::clock`'s head): whether this
        // pulse is quantised is read BEFORE `advance` overwrites the previous
        // wake's floor, so the first pulse after a lift is quantised too.
        let floored = self.clock.floored_now_or_then();
        let (dt_ms, clock_lagged) = self.clock.advance(wall_now_ms, &self.arena_cfg);
        // **16b cut I.3, the lattice.** Under a staff floor the task wakes on a
        // WALL interval (`Arena::next_due_ms`), because the arena is behind the
        // wall on purpose and a lattice target in arena time cannot be reached
        // by a wake that is. So the pulse itself is cut back to the substep
        // lattice: `dt` becomes the greatest lattice point at or below
        // `now + dt`, less `now` — which may be ZERO, a wake that came inside a
        // substep being a pulse of no time (the scripted drivers deliver every
        // beat in one). The excess is discarded as the clamp discards its excess.
        // Without it, the pull-in pulse on a set and every wake below the clamp
        // would add the OS timer's overshoot to arena time and never return to
        // the lattice, so due events would be applied against positions that
        // depend on the wall moment staff typed the verb (D-16-2; 16b cut U.1).
        // A floor is a multiple of `engaged_ms`, which is a multiple of the
        // substep for every shipped configuration (16b cut M.2, 16c Q24), so an
        // on-time floored wake moves exactly `min(cap, floor)` and loses nothing.
        let dt_ms = if floored {
            let lattice = self.substep_len_ms();
            let end = floor_to(self.now_ms.saturating_add(i64::from(dt_ms)), lattice);
            u32::try_from(end.saturating_sub(self.now_ms).max(0)).unwrap_or(dt_ms)
        } else {
            dt_ms
        };
        let length = u32::try_from(self.substep_len_ms()).unwrap_or(u32::MAX);
        let pieces = substeps(dt_ms, length);
        PulseProgress::Running(PulseWork {
            wall_now_ms,
            perception,
            reflex,
            shots_before,
            fire_intent_queued,
            dt_ms,
            clock_lagged,
            floored,
            pieces: pieces.into_iter(),
            ran: 0,
            ledger_full: false,
            stage: PulseStage::Integrate,
        })
    }

    /// Execute at most one original physics substep, or one bounded non-time
    /// phase. No new pulse/barrier/clock calculation is introduced by yielding.
    fn continue_pulse(&mut self, mut work: PulseWork) -> PulseProgress {
        match work.stage {
            PulseStage::Integrate => {
                if let Some(piece) = work.pieces.next() {
                    work.ledger_full |= self.substep(piece, &mut work.perception, &mut work.reflex);
                    work.ran = work.ran.saturating_add(1);
                    if self.ended.is_some() || work.pieces.len() == 0 {
                        work.stage = PulseStage::Actions;
                    }
                } else {
                    work.stage = PulseStage::Actions;
                }
            }
            PulseStage::Actions => {
                self.pulse_actions(&mut work.reflex);
                work.stage = PulseStage::Drain;
            }
            PulseStage::Drain => {
                self.pulse_drain(&mut work);
                work.stage = PulseStage::Finish;
            }
            PulseStage::Finish => return PulseProgress::Done(self.finish_pulse(work)),
        }
        PulseProgress::Running(work)
    }

    fn finish_pulse(&mut self, work: PulseWork) -> StepOutput {
        let PulseWork {
            wall_now_ms,
            perception,
            reflex,
            shots_before,
            fire_intent_queued,
            dt_ms,
            clock_lagged,
            ran,
            mut ledger_full,
            ..
        } = work;
        // ---- phase 5: the pulse's own record ----
        //
        // Doc 16 §3.1's `Alert` row: *any unit holds a contact (classified or not)
        // whose Euclidean range is under `tempo.alert_range_cm`*, *or a fire intent
        // is queued*. It is evaluated here, after phase 3 minted and expired this
        // pulse's contacts and phase 4 resolved this pulse's shots, so the tempo
        // answers to the world the pulse ended with.
        //
        // The `Engaged` row's first two triggers are one fact in this engine: doc 16
        // §3.5 makes every accepted shot schedule an impact, so a pulse that fired is
        // a pulse that scheduled one.
        //
        // **16b cut N.1 fills the other two.** Its **third** `Engaged` trigger —
        // *hostile units are within weapons range of each other* — was left `false`
        // by cut W.1 because a field-wide sweep is a cost nobody had bounded; it is
        // not a sweep, it is a read of the contacts each platform already holds
        // (`Arena::hostile_in_weapons_range`), which is the same pass the
        // controller's `Approach` and `Engage` scoring makes and is bounded by the
        // contact store rather than by the field. And the `Alert` row's third
        // trigger — *a controller has entered `Approach`* — is the controller's own
        // state, which is now a thing that exists.
        let shot_fired = self.next_shot_seq > shots_before;
        let signals = TempoSignals {
            fire_intent_queued,
            contact_within_alert_range: self.contact_within_alert_range(),
            // 17b cut M4.3: a mind's `Approach` off its cached doctrine state
            // (`Arena::any_mind_approaching`) — since 17b cut M4.6 the only
            // machine's, and read only on a hull sealed on Standard Doctrine
            // (`SealRef::doctrine`).
            ai_approaching: self.any_mind_approaching(),
            shot_fired,
            impact_scheduled: shot_fired,
            hostile_in_weapons_range: self.hostile_in_weapons_range(),
        };
        //
        // **16b cut M.2**: a match a substep already ended moves no tempo and
        // journals no `tempo` or `lagged` line. Cut W.2's rule is that nothing is
        // journalled after `match_ended`, and with the duration bound settled
        // inside the substep (16c `T-01`) a timeout is now such an end too: the
        // shot in flight that the deadline cut off would otherwise raise
        // `Engaged` a line after the match was over.
        let over = self.ended.is_some();
        let changed = if over {
            None
        } else {
            self.clock
                .evaluate(self.now_ms, &signals, &self.arena_cfg.tempo)
        };
        // One hash for every event this phase appends, taken after the tempo is
        // settled so that a `tempo` event carries the tempo it announces.
        //
        // **16c T-104 (the walks)**: taken LAZILY, at the first line this phase
        // journals, and shared by every later line of the phase — the same
        // value the eager walk read, because nothing between the tempo's
        // settling and the first line writes hashed state (the floor change
        // and the lag are views, the settles write `ended`, which is not
        // folded), and the lines' own appends come after the slot is filled,
        // exactly as they came after the eager walk. A wake that journals
        // none of these lines — nearly every wake — now walks the state once
        // less: the VPS profile read this walk at 8 per cent of the CI step.
        let mut wake_hash: Option<u64> = None;
        if let Some((from, to)) = changed {
            let hash = self.wake_hash(&mut wake_hash);
            ledger_full |= !self.journal(self.now_ms, EventKind::Tempo { from, to }, hash);
        }
        // **16b cut I.3** (16c N10): a staff floor set or lifted since the last
        // pulse, once per change, beside the `tempo` line and under the same
        // hash. The floor is a view of the wake cadence and no hash folds it;
        // this line moves the ledger's sequence like every line (`hash.rs` item
        // 11). Not on a match a substep already ended: nothing is journalled
        // after `match_ended` (16b cut W.2), and the change is left untaken.
        if !over && let Some(period_ms) = self.clock.take_floor_change() {
            let hash = self.wake_hash(&mut wake_hash);
            ledger_full |= !self.journal(self.now_ms, EventKind::TempoFloor { period_ms }, hash);
        }
        // One notice per pulse whichever half is behind: the clock absorbed more
        // wall time than the clamp allows (doc 16 §3.1 "Clock and lag"), or the
        // egress is backing up past `ledger.max_unflushed_chunks` (§3.1's egress
        // paragraph). `dt_ms` is the clamped `dt` the pulse actually ran with.
        let egress_lagged = self.ledger.lagged();
        if !over && (clock_lagged || egress_lagged) {
            let hash = self.wake_hash(&mut wake_hash);
            ledger_full |= !self.journal(self.now_ms, EventKind::Lagged { dt_ms }, hash);
        }
        // **16b cut W.2**, the **backstop** for doc 16 §3.8's team elimination. Every
        // ordinary elimination is settled inside the substep the last unit went out of
        // action in (see `Arena::substep`), because that is what makes its timestamp a
        // fact of arena time rather than of the cadence; `settle_elimination` answers
        // `None` for a match that has already ended, so this arm fires only for a side
        // put out by something that is not a substep — a direct `destroy_unit` from a
        // test rig today, and whatever a later cut adds. It is **before** the duration
        // check, so a match whose last unit dies on the pulse the clock runs out is a
        // victory and not a timeout.
        //
        // **16b cut Q.7** put `Spent` beside it, in that order: a side eliminated
        // on the same pulse is an elimination with a winner, and a draw over the
        // top of it would lose the result.
        //
        // **16b cut O.1 adds no Hold backstop here, on purpose.** A Hold win is
        // settled by its own due event inside a substep (`DueKind::HoldSettles`),
        // and nothing outside a substep moves a pose or wrecks a unit on a
        // schedule the clock could miss: a staff `@arena/place` moves a pose that
        // the next substep's end reads, and a direct `destroy_unit` from a rig
        // is read by the next substep's presence settle and by the event's own
        // fresh read at its pop.
        let mut settled = self.settle_elimination();
        if settled.is_none() {
            settled = self.settle_spent();
        }
        // **17b cut M3.6**: each backstop end below closes every open sortie
        // first, at the end's millisecond under the end's hash
        // (`Arena::journal_sortie_closes`), so the end stays the last line.
        if let Some(end) = settled {
            let hash = self.wake_hash(&mut wake_hash);
            ledger_full |= !self.journal_sortie_closes(self.now_ms, hash);
            ledger_full |= !self.journal(
                self.now_ms,
                EventKind::MatchEnded { reason: end.reason },
                hash,
            );
        }
        // **16b cut M.2** (16c `T-01`): the duration bound is a due event now
        // (`DueKind::MatchDeadline`), settled inside the substep at its own
        // millisecond, so this check is the **backstop** and fires only on a pulse
        // that ran no substep — the opening pulse of a match whose
        // `max_duration_ms` is `0`, which AA.3's stalled-terminal proof relies on
        // and which the phase-4 drain never touches. On a pulse whose deadline the
        // due loop already settled, `ended` is set and this is skipped.
        if self.ended.is_none() && self.now_ms >= i64::from(self.match_cfg.max_duration_ms) {
            let reason = EndReason::Timeout;
            let hash = self.wake_hash(&mut wake_hash);
            ledger_full |= !self.journal_sortie_closes(self.now_ms, hash);
            ledger_full |= !self.journal(self.now_ms, EventKind::MatchEnded { reason }, hash);
            self.ended = Some(MatchEnd {
                reason,
                t_ms: self.now_ms,
            });
        }
        // Doc 16 §4: at 100% of `ledger.max_bytes` the match is stopped with
        // `Timeout`. There is no event to append — the ledger is exactly what has
        // run out of room — so the end is recorded in the result and not in the
        // black box. **17b cut M3.6**: and no sortie closes on the tape here
        // either (declared): there is no room for its line, and the end has
        // none for the closes to precede.
        if self.ended.is_none() && ledger_full {
            self.ended = Some(MatchEnd {
                reason: EndReason::Timeout,
                t_ms: self.now_ms,
            });
        }
        // The world cannot drain the egress and the match is **void** — nothing it
        // derived may be committed. `EndReason::LedgerOverflow` is A.1's declared
        // name for exactly this condition and the `void` event is what marks the
        // match void.
        //
        // Doc 16 §3.1: *past `ledger.max_unflushed_chunks` the match is stopped with
        // `Void`*. `Ledger::overflowed` is that bound exactly, and `Ledger::lagged`
        // (half of it) is the warning journalled before the match is lost; ruled at
        // the review of cut A.2, so the arena tolerates the documented depth and no
        // more.
        if self.ended.is_none() && self.ledger.overflowed() {
            // 17b cut M3.6: a void closes every open sortie too, first.
            let hash = self.wake_hash(&mut wake_hash);
            let _ = self.journal_sortie_closes(self.now_ms, hash);
            let _ = self.journal(self.now_ms, EventKind::Void, hash);
            self.ended = Some(MatchEnd {
                reason: EndReason::LedgerOverflow,
                t_ms: self.now_ms,
            });
        }
        let ended = self.ended;
        if ended.is_some() {
            // The last chunk leaves with the match rather than with the next
            // pulse, which there is not going to be.
            self.ledger.flush();
        }
        let tempo = self.clock.tempo();
        let frames = self.frames(tempo);
        // **16b cut I.4** (16c N10): the staff picture, after the pilots' frames
        // and from the same committed state, while anybody watches — at most
        // once per pulse and at most once an arena second, and on the pulse the
        // match ended whatever second it is, so a watcher sees the end. The gate
        // is a view (`Arena::watched`, `Arena::last_observer_s`) that no hash
        // folds and nothing that steers reads, so a watched match is the same
        // match at every tempo; the one cost is this `bool` read on a pulse
        // nobody watches.
        let observer = if self.watched
            && (self.last_observer_s != Some(self.now_ms.div_euclid(1_000)) || ended.is_some())
        {
            self.last_observer_s = Some(self.now_ms.div_euclid(1_000));
            Some(self.observer())
        } else {
            None
        };
        StepOutput {
            ledger_chunks: self.ledger.take_chunks(),
            frames,
            tempo,
            next_due_ms: self.next_due_ms(wall_now_ms),
            lagged: self.clock.lagged() || self.ledger.lagged(),
            ended,
            substeps: ran,
            perception,
            motor: reflex,
            // **16b cut N.2** (doc 16 §6): the last thing the pulse does, after the
            // frames, so a signal describes the world the pilots were just shown.
            // A match that ended this pulse asks for nothing: an order for a match
            // that is over is a provider call nobody will read.
            deliberation: if ended.is_some() {
                Vec::new()
            } else {
                self.take_deliberation_signals()
            },
            observer,
            // **17b cut M4.1**: the last substep's thinks, for the task's
            // pool. A match that ended this pulse hands out none: nothing is
            // written after the end.
            thinks: if ended.is_some() {
                Vec::new()
            } else {
                self.unsettled.clone()
            },
        }
    }

    /// **One pulse, with its thinks settled** (17b cut M4.1; doc 17 §6
    /// "Execution policy"): [`Arena::step`], then every think the pulse left
    /// outstanding run inline on this thread
    /// ([`crate::arena::mind::run_batch_inline`]) and written
    /// ([`Arena::deliver_answers`]) — the pooled path done synchronously,
    /// writing at the same sim-time moment the sim task's delivery writes
    /// (before anything of the next pulse) and that the next `step`'s head
    /// would. The faults are folded into the output's counters, and
    /// [`StepOutput::thinks`] is empty: nothing is left for a pool. What
    /// every rig that hashes, encodes or reads a mind row BETWEEN two pulses
    /// drives, so the arena between two of its pulses is the arena between
    /// two of the task's.
    pub fn step_settled(
        &mut self,
        wall_now_ms: i64,
        controls: &[Control],
        intents: &[Intent],
    ) -> StepOutput {
        let mut out = self.step(wall_now_ms, controls, intents);
        let thinks = std::mem::take(&mut out.thinks);
        let settled = self.deliver_answers(crate::arena::mind::run_batch_inline(&thinks));
        out.motor.mind_faults = out.motor.mind_faults.saturating_add(settled.faults);
        out.motor.mind_overflows = out.motor.mind_overflows.saturating_add(settled.overflows);
        out
    }

    /// **One substep** of doc 16 §3.1 phase 3, answering whether the ledger
    /// refused an event.
    ///
    /// (a) motion, in unit-id order, from the substep-start snapshot; (b) the index
    /// rebuilt; (c) arena time advanced and every due event at or before it
    /// applied. The events (a) and (c) produced are appended **last**, in that
    /// order — the `blocked` notices in unit-id order, then the due events in
    /// `(t_ms, seq)` order — against one hash taken at the substep's end.
    fn substep(
        &mut self,
        dt_ms: u32,
        perception: &mut PerceptionCounters,
        reflex: &mut MotorCounters,
    ) -> bool {
        let mut notices: Vec<Notice> = Vec::new();
        // Shot origins survive removal at impact until this substep's journal
        // projects their own notices. This scratch holds only resolving shots,
        // keyed by the actual shot rather than the shooter's latest launch.
        let mut shot_origins = BTreeMap::new();
        // **16b cut F.2**: the stops this substep's motion produced, in unit-id
        // order, collected inside the borrow of `self.units` and staged after it.
        // A pilot's blocked notice is staged here and not from the journal scan
        // below the way every other one is; since 16b cut M.3 it fires on exactly
        // the edge the ledger's own `blocked` line does (`MotionOutcome::held_edge`),
        // at the same point in the same order, which is what the scan would have
        // given.
        let mut stops: Vec<(ObjId, crate::arena::motion::BlockedCause)> = Vec::new();
        // **16b cut Q.7**: the same stops again, with the cell, for the hull's
        // remembered stops (the motor row's since 17b cut M1a.1, the reflex
        // controller's memory before) — the same edge once more (16b cut M.3).
        // They are collected here and applied below — after arena time has
        // advanced, so the timestamp a row remembers is the one the `blocked`
        // ledger line carries, and before the due-event loop, so a motor tick
        // of this very substep already knows (as the reflex's tick did until
        // 17b cut M4.6).
        let mut learned: Vec<(ObjId, (u16, u16), crate::arena::motion::BlockedCause)> = Vec::new();

        // **16b cut E.3** (16c `B-44`), before motion: a machine steering at a
        // goal with no route is steering at a direction nobody has checked, and
        // until this cut it only found that out at its own decide tick — once a
        // second of arena time, against eight substeps of driving. The pre-pass
        // asks the crossing's own question of the cell in front of every such
        // machine and orders the ones that would be refused to stop, through the
        // same door a pilot's `SPEED 0` goes through.
        //
        // **16b cut M.2**: neither it nor motion runs in a zero-length substep —
        // the phase-4 drain (`Arena::step`), which exists to pop what phase 4
        // scheduled at `now_ms` and nothing else. No time passes in it, so there
        // is no motion to integrate, and `integrate` at `dt 0` is not a pure
        // no-op: it clears the per-substep `blocked` flag. (Until 16b cut M.3 that
        // flag's rising edge was the `blocked` line, so a drain would have made
        // the next real substep's re-collision with the same bank a second line;
        // the line now fires on `held`'s edge, which only a refusal raises, and
        // the guard stands because there is still nothing to move.)
        // **17b cut M4.1** (doc 17 §6 "Execution policy", D-UM-22): the head
        // of the substep is where the thinks of the LAST substep are written —
        // their runners called now, inline, when the sim task did not deliver
        // their answers before this step (it delivers only between pulses, so
        // inside a pulse of several substeps this is the path, counted in the
        // wake). Before motion: the write is a function of the think's frozen
        // inputs and the row, and the landing one substep on reads it.
        let settled = self.settle_inline();
        reflex.mind_faults = reflex.mind_faults.saturating_add(settled.faults);
        reflex.mind_overflows = reflex.mind_overflows.saturating_add(settled.overflows);

        if dt_ms > 0 {
            self.hold_routeless(reflex);
        }

        // ---- (a) motion ----
        if dt_ms > 0 {
            // **16b cut E.3**: the arena millisecond this substep *ends* at, and
            // the key that prices a traffic hold against it. Both are read before
            // the borrow below, because arena time itself is not advanced until
            // (c) — a hold opened here is stamped with the boundary the motion
            // belongs to, which is the same boundary the `blocked` notices below
            // are stamped with.
            let end_ms = self.now_ms.saturating_add(i64::from(dt_ms));
            let occupied_stop_ms = self.arena_cfg.occupied_stop_ms;
            self.index.rebuild(&self.definition.terrain, &self.units);
            // **16b cut U.2** (16c `T-29`): the jamming memo goes with the index it
            // was folded over — motion is about to move the poses it read.
            self.jam_fold.clear();
            let terrain = &self.definition.terrain;
            let index = &mut self.index;
            for slot in 0..self.units.len() {
                let Some(unit) = self.units.get_mut(slot) else {
                    continue;
                };
                let id = unit.id;
                let from = terrain.cell_of(&unit.pose);
                let outcome = integrate(unit, dt_ms, terrain, index, end_ms, occupied_stop_ms);
                if outcome.moved
                    && let Some(from) = from
                    && let Some(to) = terrain.cell_of(&unit.pose)
                    && from != to
                {
                    // Doc 16 §3.4 step 5's snapshot rule: the unit that integrates
                    // next must see where this one went, or two units enter one
                    // cell in one substep.
                    index.move_unit(id, from, to);
                }
                // **One edge for three readers** (16b cut M.3; 16c `T-04`,
                // `B-27`, §5's Q22): the ledger's `blocked` line, the pilot's
                // one notice and the controller's remembered stop all fire on
                // `UnitState::held`'s new edge — a stop at a cell or for a cause
                // the unit was not already held against — and on nothing else.
                // A tank that re-collides with the same bank every substep is
                // one stop however the pulse is cut; a bank and then an occupied
                // cell met inside the same cell after a turn are two. The
                // per-substep `blocked` flag journals nothing any more: its
                // rising edge was a function of the partition. `blocked_at` and
                // `blocked_by` are written together by `stop_at_boundary`, so the
                // cell and the cause are both in hand at the edge.
                if outcome.held_edge
                    && let (Some((x, y)), Some(cause)) = (outcome.blocked_at, outcome.blocked_by)
                {
                    notices.push((
                        0,
                        EventKind::Blocked {
                            unit: id,
                            x,
                            y,
                            cause,
                        },
                        Vec::new(),
                    ));
                    // 16b cut Q.7: the controller learns from the edge and not
                    // from the per-substep flag, which is cleared before the next
                    // tick could read it.
                    learned.push((id, (x, y), cause));
                    stops.push((id, cause));
                }
            }
        }
        for (unit, cause) in stops {
            self.stage_notice(unit, FrameNotice::Blocked { cause });
        }

        // ---- (b) the index over the positions motion committed ----
        self.index.rebuild(&self.definition.terrain, &self.units);
        // **16b cut U.2** (16c `T-29`): and the jamming memo is emptied with it.
        // From here to the next rebuild no pose moves, so a fold taken by the due
        // loop below stays true until a `StartupComplete` or an `Impact` moves a
        // pod's power or wreck — the two arms that clear it again.
        self.jam_fold.clear();

        // ---- (c) arena time, and every due event at or before it ----
        self.now_ms = self.now_ms.saturating_add(i64::from(dt_ms));
        for notice in &mut notices {
            // The `blocked` notices belong to the interval just integrated, which
            // ends here.
            notice.0 = self.now_ms;
        }
        // 16b cut Q.7: and so does what the controller remembers of them.
        for (unit, cell, cause) in learned {
            self.note_blocked(unit, cell, cause, self.now_ms);
        }

        // **16b cut X.1** (doc 16 §9 item 1): the heat the interval just
        // integrated shed, at the same boundary the motion belongs to. It is a
        // rate over **arena time** and not a thing that happens per pulse, which
        // is why it is here and not in phase 5 — and why the remainder is carried
        // rather than truncated (`crate::arena::heat`). It journals nothing: doc
        // 16 §4's event kinds are a closed list and heat is a cell, whose changes
        // are M32's own journal's to record.
        self.dissipate_heat(dt_ms, self.now_ms);
        // **16b cut M.2** (16c `T-01`): the duration bound's millisecond, once its
        // event has popped in this substep, and the loop's horizon with it.
        let mut deadline_hit: Option<i64> = None;
        // **16b cut M.2** (16c `T-83`): the held losses whose deadline popped in
        // this loop, each with how many journal entries preceded it, so the
        // staging scan below settles it at exactly its place in `(t_ms, seq)`
        // order — see the `LossSettles` arm.
        let mut settles: Vec<(usize, ObjId, ContactId, i64)> = Vec::new();
        // **16b cut O.1** (16c N11): the Hold win this loop confirmed, and its
        // millisecond — ended after the loop, before the two settles.
        let mut hold_won: Option<(Team, i64)> = None;
        // **17b cut M1a.5** (doc 17 §6, D-UM-20, D-UM-21): the mind kinds of
        // the millisecond being popped, held back — the landings until every
        // other kind due at that millisecond has popped, the thinks until the
        // landings and everything due at that millisecond after them, what the
        // landings scheduled at it included, have run (17b cut M1b.0). See the
        // loop's head below.
        let mut held: Vec<DueEvent> = Vec::new();
        // The horizon is the substep's end until the deadline pops, and the
        // deadline's own millisecond after: `pop_due` then answers only what
        // shares that millisecond (everything earlier has already been popped),
        // and the loop stops with every later event left in the heap.
        //
        // **The order of a millisecond: every landing, then the heap's ordinary
        // events at that millisecond, then the thinks** — so an impact a
        // landing's beam schedules at its own millisecond is applied before any
        // think of that millisecond takes its view (doc 17 §6, D-UM-20 as
        // amended 2026-10-04; 17b cut M1b.0, which replaced cut M1a.5's one
        // pass that ran the thinks before such an impact). D-UM-20: a think is
        // resolved *after that millisecond's other due items, including any a
        // landing of that millisecond schedules at it; the view is the state
        // then*; D-UM-21: landings *in unit-id order*.
        //
        // `DueEvent`'s order is `(t_ms, seq, kind)` with `seq` unique, so where a
        // kind sits in its enum orders nothing, and a `SensorPass` scheduled
        // after a `MindThink` at the same millisecond pops after it. So a
        // popped `MindThink` or `MindLand` is held, and popping goes on while
        // the heap's head is due at the same millisecond. When it is not — the
        // head is later, past the horizon, or the heap is empty — the held
        // events run in two phases:
        //
        // 1. while any `MindLand` is held, **the landings alone** run
        //    (`Arena::run_mind_landings`), by unit then `think_seq`, the thinks
        //    left held, and the loop goes round: whatever those landings
        //    scheduled at this millisecond is now the heap's head, none of it a
        //    mind kind, and pops before the thinks: a zero-flight `Impact`
        //    (`resolve_fire` puts a beam's `impact_at` at its own millisecond —
        //    the case this order exists for — and a shell's at a range its
        //    round crosses in under a millisecond); a goal's `MotorTick`, when the
        //    landing falls on the substep's end (`land_goal` →
        //    `Arena::engage_motor`, whose `grid_at` answers the first grid slot
        //    at or after `now_ms`, which in this loop is the substep's end);
        //    and a `StartupComplete` under a `startup_ms` of nought (legal;
        //    shipped at 10,000). A landing schedules no `Share`: only the
        //    `SensorPass` arm does. Every landing runs before any of their
        //    impacts, so two minds' beams at one millisecond are simultaneous:
        //    both fire and both impact — the second hull is no wreck yet when
        //    it lands — exactly as two reflex ticks at one millisecond were (a
        //    tick's `Impact` took a later `seq` than the other tick's event);
        // 2. when the head has left the millisecond again with only thinks
        //    held, **the thinks** run (`Arena::run_mind_thinks`), by unit then
        //    `think_seq`: unit-id order by construction, whatever their `seq`
        //    and whenever a mind was adopted. A think schedules nothing at its
        //    own millisecond (its landing one substep on, its next think its
        //    row's `think_ms` on — 17b cut M3.1; at Class III's 250 ms, one
        //    substep, the landing and the next think share a millisecond and
        //    the landing runs first, by step 1).
        //
        // **Declared asymmetry, retired** (doc 17 §24; 17b cut M1b.0): reflex
        // ticks and mind landings at one millisecond were NOT mutual. A reflex
        // tick was an ordinary event, so it ran while the landings were still
        // held and its beam's `Impact` popped before them — a reflex beam
        // killed a mind hull before it landed — and a mind's beam could not
        // pre-empt a reflex tick at that millisecond either. Reachable only with
        // laser hulls on both controllers at one millisecond; 17b cut M4.3's
        // flip (every unseated hull a mind's) made it unreachable, and 17b cut
        // M4.6 deleted the reflex tick.
        //
        // What moved against cut M1a.5's one pass, and what did not. Moved, and
        // for mind hulls only: whatever a landing schedules at its own
        // millisecond now runs before that millisecond's thinks where it ran
        // after them — the beam's `Impact` (the point), a goal's `MotorTick`
        // when the landing falls on the substep's end (a think of that
        // millisecond now sees the hull's motor step taken), a `StartupComplete`
        // under a `startup_ms` of nought. Not moved: `resolve_fire`'s
        // `WeaponReady` is at `+cycle_ms` (W14's floor of 1,000 for shipped
        // content), and `engage_motor`'s `MotorTick` off the substep's end is at
        // a later grid millisecond than the landing's; and no production unit
        // is a mind, so production never holds an event here and the loop is
        // the loop it was (the three production pins and 17b cut M1b.0's
        // line-hash pin).
        loop {
            let horizon = deadline_hit.unwrap_or(self.now_ms);
            // Every held event shares one millisecond: one is held only while
            // the head is due at the held one's own.
            if let Some(at) = held.first().map(|event| event.t_ms)
                && !self
                    .due
                    .peek_due(horizon)
                    .is_some_and(|next| next.t_ms == at)
            {
                if held
                    .iter()
                    .any(|event| matches!(event.kind, DueKind::MindLand { .. }))
                {
                    self.run_mind_landings(&mut held, reflex);
                } else {
                    self.run_mind_thinks(&mut held, reflex);
                }
                continue;
            }
            let Some(event) = self.due.pop_due(horizon) else {
                break;
            };
            if event.kind.is_mind() {
                held.push(event);
                continue;
            }
            match event.kind {
                // **16b cut M.2** (16c `T-01`; doc 16 §3.8's `Timeout`): the match's
                // duration bound, at its own arena millisecond. The arm only
                // records the millisecond; the loop's horizon then shrinks to it,
                // so an event due at the deadline's own millisecond still lands —
                // an impact scheduled there resolves, and a kill there still wins,
                // because elimination is settled below before the timeout is — but
                // an impact, pass, share or tick due AFTER the deadline and at or
                // before this substep's end is NOT applied and journals nothing.
                // Cut W.2's rule is that nothing is journalled after the match is
                // over, and without the horizon a round landing after the deadline
                // turned a timeout into an elimination. What is left in the heap
                // is never applied: the match is over.
                DueKind::MatchDeadline => {
                    deadline_hit = Some(event.t_ms);
                }
                // **16b cut O.1** (16c N11; plan items 1, 15, 40): a sole
                // occupant's clock has run out. The win is awarded only when this
                // event is the clock that is still running — `held_by` names this
                // very `(team, since_ms)`, so an occupancy that was contested or
                // re-opened since leaves a stale event that fires into nothing —
                // **and a presence reading taken FRESH, here, at the pop** still
                // finds `team` alone. `held_by` was written at the previous
                // boundary; the fresh read is over the positions this substep's
                // motion committed and every impact this loop has applied so far,
                // so a holder wrecked since, or an enemy driven into the square
                // during this substep, voids the win. A confirmed win shrinks the
                // horizon exactly as the deadline's arm does, so nothing due after
                // its millisecond is applied or journalled.
                //
                // **The three true tie rules**, each pinned by a test:
                //
                // * `HoldSettles` is scheduled at `since_ms`, at least
                //   `MIN_HOLD_MS` (10 s) before it pops, so it carries a lower
                //   sequence than any impact due at the same millisecond (a round
                //   flies at most 2.56 km at 1,000 m/s) and pops first; and
                //   `MatchDeadline`, scheduled at check-out, is lower still;
                // * (1) **a holder wrecked at an EARLIER millisecond of the last
                //   substep voids the win**: the fresh read finds nobody;
                // * (2) **a holder wrecked at the hold's OWN millisecond still
                //   wins**: the impact pops after this arm, inside the shrunk
                //   horizon, is applied and journalled, and then the match ends as
                //   `hold` — the one moment the objective was met, it was met;
                // * (3) **a hold at the deadline's own millisecond beats
                //   `Timeout`**: the hold is settled before the timeout arm below,
                //   which asks `self.ended.is_none()`.
                DueKind::HoldSettles { team, since_ms } => {
                    if self.ended.is_none()
                        && self
                            .hold
                            .as_ref()
                            .is_some_and(|hold| hold.held_by == Some((team, since_ms)))
                        && self.hold_presence() == Some(team)
                    {
                        hold_won = Some((team, event.t_ms));
                        deadline_hit = Some(event.t_ms);
                    }
                }
                // **16b cut M.2** (16c `T-83`; doc 16 §3.6 "What a pilot sees"): a
                // held loss's hysteresis has run out. **The rule**: the loss is
                // said at the first substep boundary at or after the deadline
                // unless the platform re-detected the contact at or before it. A
                // sensor pass landing exactly on the deadline is applied before
                // the expiry (it was scheduled earlier, so its sequence is lower),
                // and a mesh share landing exactly on it after (it is scheduled
                // by that pass), which is the one millisecond the two arms can
                // differ on and is the same at every cadence.
                //
                // It is **settled in the staging scan**, not here, at the position
                // the event popped at. A re-detection's `detected` is journalled
                // into `notices` by its pass and only reaches
                // `Arena::stage_pilot_notice` — which is where a held loss is
                // cancelled — in the scan below, after this whole loop; removing
                // the entry here would say a loss that a pass earlier in this same
                // substep had already cancelled. Recording the position keeps the
                // two in `(t_ms, seq)` order, and it also means every write to
                // `held_losses` happens in the scan, after the substep's hash, so
                // each is first folded at *k + 1* (the `spent_said` rule).
                DueKind::LossSettles { viewer, contact } => {
                    settles.push((notices.len(), viewer, contact, event.t_ms));
                }
                // Doc 16 §3.3 "Startup": `match.startup_ms` after `STARTUP` was
                // accepted. `resolve_startup` already journalled the transition
                // into `starting`.
                //
                // **16b cut Q.1** gave the far end its own event. Doc 16 §4's
                // kind list had no `power_online`, so the moment a tank became
                // driveable was the one transition in the engine nothing recorded
                // and nobody was told about: the owner's first hand-played match
                // found it by typing `HEADING` until one of them stopped being
                // refused. The line is journalled at the **event's** timestamp
                // and only on the real flip, so a startup overtaken by a wreck
                // writes nothing, and `stage_pilot_notice` projects the pilot's
                // `Power online.` off it exactly as every other notice is a
                // projection of a journalled event.
                DueKind::StartupComplete { unit } => {
                    if let Some(state) = self.unit_mut(unit)
                        && state.power == Power::Starting
                    {
                        state.power = Power::Online;
                        notices.push((event.t_ms, EventKind::PowerOnline { unit }, Vec::new()));
                        // **17b cut M1b.1** (doc 17 §5, D-UM-16): a slot armed
                        // while the hull was cold fires at the millisecond its
                        // power comes up, not at the next pass. A hull with no
                        // slot returns at once and touches nothing.
                        let _ = self.evaluate_conditions(
                            unit,
                            event.t_ms,
                            ConditionTrigger::Startup,
                            None,
                            reflex,
                        );
                    }
                    // **16b cut U.2** (16c `T-29`): a pod that has just come online
                    // radiates from this millisecond, so every fold taken earlier in
                    // this substep is stale. Cleared whether or not the flip
                    // happened: cheaper to ask again than to reason about it.
                    self.jam_fold.clear();
                }
                // Doc 16 §4: every unit, every `ledger.summary_ms` of arena time,
                // in every tempo. The next one is scheduled from **this one's
                // timestamp** and not from `now_ms`, so the cadence is a function
                // of arena time alone and cannot drift with the substep boundary
                // the summary happened to land on. It is rescheduled even when the
                // 90% bound has silenced it, so summaries resume if the bound is
                // raised.
                DueKind::PositionSummary => {
                    if self.ledger.summaries_enabled() {
                        notices.push((
                            event.t_ms,
                            EventKind::PositionSummary {
                                units: self.position_summary(),
                            },
                            Vec::new(),
                        ));
                    }
                    let cadence = i64::from(self.ledger.summary_ms().max(1));
                    self.due
                        .schedule(event.t_ms.saturating_add(cadence), DueKind::PositionSummary);
                }
                // **16b cut S.1** (doc 16 §3.6): one unit's pass, applied at the
                // event's own timestamp against the positions this substep
                // committed. The next pass is scheduled from **this one's
                // timestamp** and not from `now_ms`, exactly as the position
                // summary above is, so the cadence is a function of arena time
                // alone and cannot drift with the substep boundary the pass
                // happened to land on.
                //
                // A pass that leaves the unit holding **anything the mesh may
                // carry** — a classified contact its own sensors saw inside the
                // time to live, which is 16b cut F.3's two share rules and is
                // written once in `ShadowContact::shareable`; `Arena::sensor_pass`'s
                // head has the argument — schedules a `Share` at the *same*
                // timestamp: the heap orders
                // `(t_ms, seq, kind)` and the share is inserted with a later
                // sequence, so it is applied after the pass, in this same substep,
                // and the mesh never lags a pulse behind the tracks it carries. It
                // schedules nothing itself, so the loop terminates.
                DueKind::SensorPass { unit, pass_seq } => {
                    let shared =
                        self.sensor_pass(unit, pass_seq, event.t_ms, perception, &mut notices);
                    if shared {
                        self.due.schedule(event.t_ms, DueKind::Share { unit });
                    }
                    // **17b cut M1b.1** (doc 17 §5: *at every sensor pass that
                    // refreshes the conditioned contact's plot*; 17b §1(d)): when
                    // this hull's OWN sighting wrote its lock's row at this
                    // millisecond, its slots read the new plot now. A hull with
                    // no slot asks one map miss and nothing else.
                    if !self.conditions.is_empty_for(unit)
                        && (self.own_pass_wrote_lock(unit, event.t_ms)
                            || self.has_sensor_rules(unit))
                    {
                        let _ = self.evaluate_conditions(
                            unit,
                            event.t_ms,
                            ConditionTrigger::Pass,
                            None,
                            reflex,
                        );
                    }
                    // **16b cut F.3**: rescheduled for every unit still in the
                    // match, not only for one that carries a sensor. The pass is
                    // where contacts expire, and a platform with no sight of its
                    // own still holds what the mesh handed it
                    // (`Arena::schedule_first_passes` has the argument).
                    if self.unit(unit).is_some() {
                        let cadence = i64::from(self.sensors_cfg.pass_ms.max(1));
                        self.due.schedule(
                            event.t_ms.saturating_add(cadence),
                            DueKind::SensorPass {
                                unit,
                                pass_seq: pass_seq.saturating_add(1),
                            },
                        );
                    }
                }
                // **16b cut S.1** (doc 16 §3.6 step 5, W6).
                DueKind::Share { unit } => {
                    let refreshed = self.share_pass(unit, event.t_ms, perception, &mut notices);
                    // **17b cut M1b.1** (17b §1(c), (d)): each receiver holding a
                    // slot whose LOCK this share inserted or refreshed reads the
                    // new plot at this millisecond — in unit-id order, once each.
                    // `share_pass` answered pairs only for receivers holding a
                    // slot, so a production share hands back an empty vector.
                    if !refreshed.is_empty() {
                        let mut receivers: Vec<ObjId> = refreshed
                            .iter()
                            .filter(|(mate, contact)| {
                                self.unit(*mate)
                                    .is_some_and(|state| state.lock == Some(*contact))
                                    || self.has_sensor_rules(*mate)
                            })
                            .map(|(mate, _)| *mate)
                            .collect();
                        receivers.sort_unstable();
                        receivers.dedup();
                        for mate in receivers {
                            let _ = self.evaluate_conditions(
                                mate,
                                event.t_ms,
                                ConditionTrigger::Pass,
                                None,
                                reflex,
                            );
                        }
                    }
                }
                // **16b cut W.1** (doc 16 §3.5 "Scheduled impact"): the shot arrives
                // at its own timestamp, against the positions this substep
                // committed, which is what makes a moving target's escape a fact of
                // arena time and not of the pulse rate. **Cut W.2** gave it its body
                // — the four re-validations, the draw, the damage plan and the
                // consequences (`crate::arena::damage`) — and the draw is keyed on the
                // shot's sequence, so which pulse the round lands in cannot change
                // whether it hits.
                DueKind::Impact { shot_seq } => {
                    if let Some(cell) = self.shots.get(&shot_seq).and_then(|shot| shot.from_cell) {
                        shot_origins.insert(shot_seq, cell);
                    }
                    self.resolve_impact(shot_seq, event.t_ms, &mut notices);
                    // **16b cut U.2** (16c `T-29`): a round can wreck the hull a
                    // pod rides on, which turns its power off and stops it
                    // radiating (`Arena::destroy_unit`), so a fold taken before
                    // it may count a pod that is gone.
                    self.jam_fold.clear();
                }
                // **16b cut W.1**: the weapon's cycle has run down. It changes no
                // state — readiness is `ready_at_ms <= now_ms`, a comparison — and
                // it is scheduled for `next_due_ms`: it makes the arena pulse at the
                // first substep boundary at or after the millisecond the weapon
                // comes ready (16b cut U.1: within one substep of it, 250 ms on the
                // shipped content), so the pilot's frame says "ready" then rather
                // than at the next period boundary.
                //
                // **17b cut M1b.1** (doc 17 §5, D-UM-16): and it is where a
                // condition slot on that weapon is evaluated — the shot decision
                // written down fires at the millisecond the gun comes ready, not
                // at the pulse boundary after it, at every cadence. Only that
                // weapon's slot; a hull with no slot returns at once and touches
                // nothing, so the arm still changes no state for a match with no
                // slot (every production match).
                DueKind::WeaponReady { unit, ref weapon } => {
                    if self.ended.is_none() {
                        let _ = self.evaluate_conditions(
                            unit,
                            event.t_ms,
                            ConditionTrigger::WeaponReady,
                            Some(weapon),
                            reflex,
                        );
                    }
                }
                // **17b cut M1a.4** (doc 17 §4 D-UM-3): the motor's own cadence
                // — a pilot's `DRIVE`, a mind's `goto` — at its own arena
                // timestamp against the positions this substep committed, which
                // is what makes the drive rate-independent and is why it is a
                // due event and not pulse phase 4 (doc 16 §3.1's D-16-2; the
                // reflex controller's tick ran on the same argument until 17b
                // cut M4.6 deleted it, with its arm here). Nothing after the
                // match ended; a hull that is gone, a wreck or a row with no
                // goal is driven no more and the tick is not rescheduled, so the
                // cadence ends with the goal (arrival and no route both drop it
                // under `Fallback::Notice`, and a hull that can no longer move
                // drops it in `Arena::motor_step`). It journals nothing, so it
                // opens no action hash.
                DueKind::RuleLeaseExpired {
                    unit,
                    generation,
                    lands_ms,
                } => {
                    self.expire_rule_sample(unit, generation, lands_ms, event.t_ms, reflex);
                }
                DueKind::MotorTick { unit } => {
                    if self.ended.is_none()
                        && self.unit(unit).is_some_and(|state| !state.limits.wreck)
                        && self
                            .motors
                            .row(unit)
                            .is_some_and(|motor| motor.goal.is_some())
                    {
                        let _ = self.motor_step(unit, event.t_ms, reflex);
                        if self
                            .motors
                            .row(unit)
                            .is_some_and(|motor| motor.goal.is_some())
                        {
                            let cadence = i64::from(self.arena_cfg.reflex.decide_ms.max(1));
                            self.due.schedule(
                                event.t_ms.saturating_add(cadence),
                                DueKind::MotorTick { unit },
                            );
                        }
                    }
                }
                // **17b cut M1a.5**: held above and run by
                // `Arena::run_mind_landings` and `Arena::run_mind_thinks`
                // (17b cut M1b.0's two phases); never reached.
                DueKind::MindThink { .. } | DueKind::MindLand { .. } => {}
            }
        }
        // The lazy hash is scratch for this substep's due events exactly as phase
        // 4's is scratch for that phase; it never leaves the substep it was taken
        // in.
        self.action_hash = ActionHash::Outside;

        // **16b cut W.2** (doc 16 §3.8): team elimination is settled **here**, at the
        // substep the last unit of a side went out of action in, and not at the end of
        // the pulse. The difference is the whole of rate independence: the wreck is
        // applied at a due event's own timestamp inside a substep, so the moment the
        // side was eliminated is a fact of arena time — and a `match_ended` stamped at
        // the pulse boundary instead would carry a different timestamp at every
        // cadence.
        //
        // **16b cut Q.7** settles `Spent` here too, and after it: the last round
        // in flight lands as a due event inside this very substep, so the substep
        // that resolves it is the substep in which nobody can shoot any more.
        //
        // **16b cut O.1** (16c N11) puts the objective in front of both. First
        // who holds the key spot, over the poses motion committed and every
        // impact the loop above applied (`Arena::settle_hold_presence`, which
        // opens or drops the clock); then a win the `HoldSettles` arm confirmed,
        // at its own millisecond. The two settles below then answer `None` on
        // the ended match — elimination is *settled second*, the owner's default
        // — and so does the deadline's timeout arm, which is why a hold at the
        // deadline's own millisecond is a hold.
        //
        // **17b cut M3.6** (doc 17 §13): at each of the three, every sortie
        // still open closes FIRST — `sortie_closed { match_end }` per unit, in
        // unit-id order, at the end's own millisecond — pushed into the
        // substep's notices ahead of `match_ended`, so they carry the end's
        // hash (the substep's) and the end stays the tape's last line (W.2).
        self.settle_hold_presence();
        if let Some((winner, t_ms)) = hold_won {
            let reason = EndReason::Hold { winner };
            notices.extend(self.close_open_sorties(t_ms, SORTIE_MATCH_END));
            notices.push((t_ms, EventKind::MatchEnded { reason }, Vec::new()));
            self.ended = Some(MatchEnd { reason, t_ms });
        }
        let mut settled = self.settle_elimination();
        if settled.is_none() {
            settled = self.settle_spent();
        }
        if let Some(end) = settled {
            notices.extend(self.close_open_sorties(self.now_ms, SORTIE_MATCH_END));
            notices.push((
                self.now_ms,
                EventKind::MatchEnded { reason: end.reason },
                Vec::new(),
            ));
        }
        // **16b cut M.2** (16c `T-01`): the duration bound, at the deadline's own
        // millisecond, after the two settles above and only when neither ended
        // the match — an elimination or a `Spent` in the same substep wins, the
        // order phase 5's backstop has always had.
        if settled.is_none()
            && self.ended.is_none()
            && let Some(t_ms) = deadline_hit
        {
            let reason = EndReason::Timeout;
            notices.extend(self.close_open_sorties(t_ms, SORTIE_MATCH_END));
            notices.push((t_ms, EventKind::MatchEnded { reason }, Vec::new()));
            self.ended = Some(MatchEnd { reason, t_ms });
        }

        // ---- the substep's record ----
        let hash = state_hash(self);
        self.substep_hashes.push(hash);
        // **17b cut M4.1**: every think this substep collected carries this
        // hash — the one its fault's lines take wherever and whenever its
        // answer is written — and waits for the next settle point.
        self.stamp_collected(hash);
        let mut refused = false;
        // **16b cut F.2, narrowed by 16b cut D.4 (`T-08`)**: which impact is being
        // resolved, and it now serves `EventKind::Miss` **alone**.
        //
        // A `miss` line carries the shot's sequence and no shooter, and it may not
        // gain one: that would change the bytes of every `miss` line ever written.
        // But `EventKind::Impact` immediately precedes exactly one `hit` or `miss`
        // for the same shot and carries both, so the scan reads the impact and its
        // outcome as the pair they are — and **clears the pair the moment that
        // outcome has been staged**, which is what makes the read a read of this
        // round rather than of whatever round went last.
        //
        // Cut F.2 left it set for the rest of the substep, and two later arms —
        // the gunner's `target_section_lost` and his kill line — read it after the
        // pair had resolved. Anything that destroyed a section or a hull later in
        // the same substep and was not that round was credited to it, and since
        // cut Q.2 one of those arms writes hashed contact state. Both now read
        // `by` off their own event, which carries the shooter where a round did
        // it and `None` where nothing did.
        let mut in_flight: Option<(ObjId, ObjId)> = None;
        // 16b cut M.2: the held losses that came due, each settled just before the
        // first journal entry that popped after it (`LossSettles`' arm).
        let mut settles = settles.into_iter().peekable();
        // **16b cut M.2** (the M.2 review): and the held losses whose window
        // closes inside THIS substep — staged by this very scan with a deadline at
        // or before `now_ms`, which only a `[tacsim.sensors].pass_ms` shorter than
        // the substep can make (config allows 50 against a 250 ms substep). A due
        // event for one would be scheduled into the past: it would pop a boundary
        // late, and at a pulse's end the phase-4 drain would run a substep for it
        // at one cadence that another cadence never runs — two hash sequences.
        // So it never reaches the heap: it is settled here, before the first
        // entry of this scan whose millisecond is past the deadline (an entry AT
        // the deadline is a re-detection "at or before" it and goes first), which
        // is exactly the place `(t_ms, seq)` order would have popped it — its
        // event would have been scheduled after every event this loop popped, so
        // it would sort after all of them on its own millisecond.
        let mut closing: Vec<(i64, ObjId, ContactId)> = Vec::new();
        // Everything to settle before the next entry, ordered by millisecond and,
        // on one millisecond, a due event's settle before a closing window's.
        let mut due_now: Vec<(i64, u8, ObjId, ContactId)> = Vec::new();
        for (index, (t_ms, kind, draws)) in notices.into_iter().enumerate() {
            while let Some((_, viewer, contact, at)) =
                settles.next_if(|(before, ..)| *before <= index)
            {
                due_now.push((at, 0, viewer, contact));
            }
            closing.retain(|&(at, viewer, contact)| {
                if at < t_ms {
                    due_now.push((at, 1, viewer, contact));
                    false
                } else {
                    true
                }
            });
            self.settle_held_losses(&mut due_now);
            // 16b cut S.2: the pilot's notice and the ledger's event come off the
            // same journal entry, in the same order, so a line a pilot read is a
            // line the black box can be searched for.
            let launch_cell = match &kind {
                EventKind::Hit { shot_seq, .. } | EventKind::Miss { shot_seq, .. } => {
                    shot_origins.remove(shot_seq)
                }
                _ => None,
            };
            if let Some(window) = self.stage_pilot_notice(t_ms, &kind, &mut in_flight, launch_cell)
            {
                closing.push(window);
            }
            refused |= !self.journal_drawn(t_ms, kind, draws, hash);
        }
        due_now.extend(
            settles
                .map(|(_, viewer, contact, at)| (at, 0, viewer, contact))
                .chain(
                    closing
                        .into_iter()
                        .map(|(at, viewer, contact)| (at, 1, viewer, contact)),
                ),
        );
        self.settle_held_losses(&mut due_now);
        refused
    }

    /// The world asked the arena to stop (doc 16 §3.1's `Stop`): the reason is
    /// recorded, the match ends, and the ledger's tail leaves with it.
    ///
    /// The reasons journal different things, which is doc 16 §4's kind list read
    /// literally: `Abort` is an `aborted` event, `Timeout` is
    /// `match_ended{timeout}`, and `Void` is a `void` event — the one that says
    /// nothing the match derived may be committed.
    ///
    /// **16b cut D.3 (`B-33`) added two ends that are ends**: a concede and an
    /// unattended match both journal `match_ended` with their own reason, exactly
    /// as a timeout does, because both are matches that ran to a conclusion and
    /// whose cells the world commits. A concede names the side that did **not**
    /// give up, so the reader of the black box does not have to invert anything.
    ///
    /// **16b cut I.3** (16c N10: *journalled always, announced never*): a staff
    /// `Place` among the stopping pulse's `intents` — one the task buffered
    /// before the `Stop` arrived — reaches no phase 2 and no phase 4, so it is
    /// answered here, `place_refused` with the reason `stopped`, BEFORE the stop's
    /// own line (nothing follows `match_ended`, 16b cut W.2) and under the same
    /// hash, the stop pulse's one moment. Every other order in the pulse is
    /// dropped silently as it always was: the match is over and nobody is told.
    fn stopped(&mut self, wall_now_ms: i64, stop: StopReason, intents: &[Intent]) -> StepOutput {
        let hash = state_hash(self);
        for intent in intents {
            if let IntentKind::Place { x, y } = intent.kind {
                let _ = self.journal(
                    self.now_ms,
                    EventKind::PlaceRefused {
                        unit: intent.unit,
                        x,
                        y,
                        reason: SmolStr::new_static(PLACE_STOPPED),
                    },
                    hash,
                );
            }
        }
        let (kind, reason) = match stop {
            StopReason::Abort => (EventKind::Aborted, EndReason::Abort),
            StopReason::Timeout => (
                EventKind::MatchEnded {
                    reason: EndReason::Timeout,
                },
                EndReason::Timeout,
            ),
            StopReason::Void => (EventKind::Void, EndReason::Void),
            // 16b cut D.3: the side that conceded is the side that lost.
            StopReason::Concede { by } => {
                let reason = EndReason::Conceded { winner: by.other() };
                (EventKind::MatchEnded { reason }, reason)
            }
            StopReason::Unattended => (
                EventKind::MatchEnded {
                    reason: EndReason::Unattended,
                },
                EndReason::Unattended,
            ),
        };
        // **17b cut M3.6** (doc 17 §13): every open sortie closes before the
        // stop's own line, under the stop pulse's one hash — after any
        // `place_refused`, so the stop's line is still the last.
        let _ = self.journal_sortie_closes(self.now_ms, hash);
        let _ = self.journal(self.now_ms, kind, hash);
        let ended = MatchEnd {
            reason,
            t_ms: self.now_ms,
        };
        self.ended = Some(ended);
        self.ledger.flush();
        StepOutput {
            ledger_chunks: self.ledger.take_chunks(),
            frames: Vec::new(),
            tempo: self.clock.tempo(),
            next_due_ms: wall_now_ms,
            lagged: self.clock.lagged() || self.ledger.lagged(),
            ended: Some(ended),
            substeps: 0,
            perception: PerceptionCounters::default(),
            motor: MotorCounters::default(),
            deliberation: Vec::new(),
            // 16b cut I.4: a stopped pulse sends no pilot a frame and no
            // watcher a picture either; the world's `finish` clears every
            // watcher when the check-in lands.
            observer: None,
            // 17b cut M4.1: the head of this step settled every think the
            // last pulse left, and a stopped pulse collects none.
            thinks: Vec::new(),
        }
    }

    /// Append one event, answering whether it was recorded. A `false` is the
    /// ledger's byte bound (doc 16 §4), which phase 5 turns into the end of the
    /// match; it is never an error the caller has to handle mid-substep, because a
    /// black box out of tape is not a reason to lose the pulse in progress.
    fn journal(&mut self, t_ms: i64, kind: EventKind, hash: u64) -> bool {
        self.ledger.append(t_ms, kind, Vec::new(), hash).is_ok()
    }

    /// **Journal every open sortie's close at an end journalled outside a
    /// substep** (17b cut M3.6): [`Arena::close_open_sorties`] with
    /// [`SORTIE_MATCH_END`] at `t_ms`, each line under `hash` — the end's own
    /// — answering whether every line was recorded (the ledger's byte bound,
    /// as [`Arena::journal`] answers it).
    /// **The wake's tail-phase hash, taken once and only when a line needs
    /// it** (16c `T-104`, the walks). The slot is the caller's local; the
    /// first call fills it with [`crate::arena::hash::state_hash`] and every
    /// later call reads it back, so the lines of one phase share one hash as
    /// they always did, and a phase that journals nothing walks nothing.
    fn wake_hash(&self, slot: &mut Option<u64>) -> u64 {
        *slot.get_or_insert_with(|| state_hash(self))
    }

    fn journal_sortie_closes(&mut self, t_ms: i64, hash: u64) -> bool {
        let mut recorded = true;
        for (at, kind, draws) in self.close_open_sorties(t_ms, SORTIE_MATCH_END) {
            recorded &= self.journal_drawn(at, kind, draws, hash);
        }
        recorded
    }

    /// The same, with the draws doc 16 §4 asks an event to carry *where they
    /// occurred* — cut S.1's `classified` was the first event that had one, and
    /// 16b cut M.3's `hit` is the first with two (16c `T-07`).
    fn journal_drawn(
        &mut self,
        t_ms: i64,
        kind: EventKind,
        draws: Vec<(String, u64)>,
        hash: u64,
    ) -> bool {
        self.ledger.append(t_ms, kind, draws, hash).is_ok()
    }
}
