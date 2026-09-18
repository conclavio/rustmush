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

use crate::arena::clock::{Tempo, TempoSignals, substep_ms, substeps};
use crate::arena::hash::state_hash;
use crate::arena::index::range_cm;
use crate::arena::intents::{
    Control, Intent, IntentKind, IntentOutcome, apply_control, apply_intent, drain_pending,
    resolve_startup,
};
use crate::arena::ledger::{EventKind, LedgerChunk};
use crate::arena::motion::integrate;
use crate::arena::perception::{
    FrameContact, FrameNotice, Notice, NoticeKind, PerceptionCounters, bearing_mdeg,
};
use crate::arena::state::{
    Arena, Classification, ContactId, DueKind, EndReason, MatchEnd, Power, StopReason, Team,
};

/// **One pilot's latest-wins view** (doc 16 §3.1's `Frames`).
///
/// The minimal slot cut A can honestly fill: where the tank is, how fast it is
/// going, whether its power is up, whether it is held against something, and the
/// tempo the arena is running at. Cut K builds the real cockpit keyframe and delta
/// on exactly this seam, and cut S is what makes a frame *per observer* rather
/// than per unit.
///
/// It is not `PartialEq`: [`Frame::same_as`] compares the fields that decide
/// whether a pilot is told anything, and deliberately not the pose's opaque
/// posture — a frame is a view, and comparing an opaque rules blob every pulse for
/// every pilot would be the most expensive thing in phase 5.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The tank the pilot is in.
    pub unit: ObjId,
    /// **The side it is on** (16b cut S.2), so the cockpit can draw a classified
    /// contact as a friend or as a hostile without asking the world which team the
    /// pilot's slot was on. The platform's own side is never a secret from its
    /// pilot.
    pub team: Team,
    /// Where it is.
    pub pose: Pose,
    /// How fast, cm/s; negative is reverse.
    pub speed_cms: i32,
    /// Whether its power is up.
    pub power: Power,
    /// Whether it is held against an obstacle, an occupied cell or the window's
    /// edge.
    pub blocked: bool,
    /// The arena's tempo, so a cockpit can say how often it is being told things.
    pub tempo: Tempo,
    /// **What the platform holds** (16b cut S.1; doc 16 §3.6 "What a pilot sees"),
    /// in contact-id order — the same order [`crate::arena::state::ShadowContacts`]
    /// iterates in, so two pulses that hold the same set produce the same list.
    ///
    /// It is the platform's contacts and not the observer's: doc 16 §3.7 puts the
    /// station on the tank, so two pilots in one tank see one scope. Cut S.2
    /// renders it; nothing about an undetected unit is in it, which is the whole
    /// observer rule on this surface.
    pub contacts: Vec<FrameContact>,
    /// **The longest reach of anything the platform carries**, centimetres (16b
    /// cut S.2), which is what turns a contact's range into doc 16 §3.6's *range
    /// band*. Zero for a platform that carries no weapon, and then nothing is in
    /// range of it, which is the honest answer.
    pub weapon_reach_cm: u32,
    /// **What the pilot is told once** (16b cut S.2; doc 16 §3.6: *notices fire
    /// once per transition with hysteresis*), in the order the transitions
    /// happened.
    ///
    /// Deliberately outside [`Frame::same_as`]: a frame is sent when its numbers
    /// changed *or* when it carries a notice, and the copy remembered in
    /// `last_frames` has this emptied, so a notice is delivered exactly once and a
    /// later `CONTACTS` re-rendering the remembered frame does not repeat it.
    pub notices: Vec<FrameNotice>,
}

impl Frame {
    /// Whether this frame says the same thing as `other`: the unit, the four pose
    /// integers, the speed, the power, the block, the tempo and the contact list.
    ///
    /// The contacts are compared **in full**, age included, because a contact's age
    /// is one of the columns `CONTACTS` prints; a pilot parked beside a held target
    /// therefore gets a frame per pulse, which is the price of the age column being
    /// live rather than a lie.
    ///
    /// The notices are **not** compared: see [`Frame::notices`].
    #[must_use]
    pub fn same_as(&self, other: &Frame) -> bool {
        self.unit == other.unit
            && self.team == other.team
            && self.weapon_reach_cm == other.weapon_reach_cm
            && self.pose.x_cm == other.pose.x_cm
            && self.pose.y_cm == other.pose.y_cm
            && self.pose.z_cm == other.pose.z_cm
            && self.pose.heading_mdeg == other.pose.heading_mdeg
            && self.speed_cms == other.speed_cms
            && self.power == other.power
            && self.blocked == other.blocked
            && self.tempo == other.tempo
            && self.contacts == other.contacts
    }
}

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
    /// `wall_now_ms` was given in: the earlier of the next period boundary and the
    /// next due event.
    pub next_due_ms: i64,
    /// The pulse absorbed more wall time than the clamp allows, or the egress is
    /// backing up.
    pub lagged: bool,
    /// Set once, on the pulse the match ends.
    pub ended: Option<MatchEnd>,
    /// How many substeps phase 3 ran. Zero on the opening pulse, which only
    /// establishes the wall reference.
    pub substeps: u32,
    /// **What perception cost this pulse** (16b cut S.1; doc 16 §2 W4 and W6). The
    /// harness asserts `harness.max_traces_per_pulse` and
    /// `harness.max_share_ops_per_pulse` against it.
    pub perception: PerceptionCounters,
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
        self.injected_fault();
        if let Some(ended) = self.ended {
            return StepOutput {
                ledger_chunks: Vec::new(),
                frames: Vec::new(),
                tempo: self.clock.tempo(),
                next_due_ms: wall_now_ms,
                lagged: false,
                ended: Some(ended),
                substeps: 0,
                perception: PerceptionCounters::default(),
            };
        }
        self.substep_hashes.clear();
        let mut perception = PerceptionCounters::default();

        // ---- phase 1: authority, and the world's `Stop` ----
        //
        // Controls first, and every control before any intent: the epoch table
        // this phase commits is what phase 2 authorises against, which is the
        // whole mechanism behind doc 16 §3.1's `FIRE` then `EXIT` burst.
        for control in controls {
            apply_control(self, control);
        }
        if let Some(stop) = self.stop {
            return self.stopped(wall_now_ms, stop);
        }

        // ---- phase 2: orders ----
        let mut fire_intent_queued = false;
        for intent in intents {
            let outcome = apply_intent(self, intent);
            if outcome == IntentOutcome::Queued && matches!(intent.kind, IntentKind::Fire { .. }) {
                fire_intent_queued = true;
            }
        }

        // ---- phase 3: time, in substeps ----
        let (dt_ms, clock_lagged) = self.clock.advance(wall_now_ms, &self.arena_cfg);
        let length = substep_ms(&self.arena_cfg.tempo, self.cell_cm(), self.max_speed_cms());
        let pieces = substeps(dt_ms, length);
        let ran = u32::try_from(pieces.len()).unwrap_or(u32::MAX);
        let mut ledger_full = false;
        for piece in pieces {
            ledger_full |= self.substep(piece, &mut perception);
        }

        // ---- phase 4: the queued discrete actions ----
        let ids: Vec<ObjId> = self.units.iter().map(|unit| unit.id).collect();
        for id in ids {
            for order in drain_pending(self, id) {
                match order.kind {
                    // The one discrete action cut A owns (doc 16 §3.3 "Startup").
                    // A refusal is the pilot's business, not the ledger's: cut K's
                    // cockpit is what turns a `Refusal` into a line.
                    IntentKind::Startup => {
                        let _ = resolve_startup(self, id);
                    }
                    // Carried as kinds, refused until their cut lands: `LOCK` is
                    // cut S's (it needs a contact to lock) and `FIRE` is cut W's
                    // (it needs a weapon to fire). Nothing is re-queued — an
                    // action is resolved or refused in the pulse it was drained
                    // in, so a bound another order needs is never held by one that
                    // cannot be resolved.
                    IntentKind::Lock { .. } | IntentKind::Fire { .. } => {}
                    // Continuous orders never queue (phase 2 applies them).
                    IntentKind::Heading { .. } | IntentKind::Speed { .. } => {}
                }
            }
        }

        // ---- phase 5: the pulse's own record ----
        //
        // Doc 16 §3.1's `Alert` row: *any unit holds a contact (classified or not)
        // whose Euclidean range is under `tempo.alert_range_cm`*. It is evaluated
        // here, after phase 3 minted and expired this pulse's contacts, so the
        // tempo answers to the contacts the pulse ended with.
        let signals = TempoSignals {
            fire_intent_queued,
            contact_within_alert_range: self.contact_within_alert_range(),
            ..TempoSignals::quiet()
        };
        let changed = self
            .clock
            .evaluate(self.now_ms, &signals, &self.arena_cfg.tempo);
        // One hash for every event this phase appends, taken after the tempo is
        // settled so that a `tempo` event carries the tempo it announces.
        let hash = state_hash(self);
        if let Some((from, to)) = changed {
            ledger_full |= !self.journal(self.now_ms, EventKind::Tempo { from, to }, hash);
        }
        // One notice per pulse whichever half is behind: the clock absorbed more
        // wall time than the clamp allows (doc 16 §3.1 "Clock and lag"), or the
        // egress is backing up past `ledger.max_unflushed_chunks` (§3.1's egress
        // paragraph). `dt_ms` is the clamped `dt` the pulse actually ran with.
        let egress_lagged = self.ledger.lagged();
        if clock_lagged || egress_lagged {
            ledger_full |= !self.journal(self.now_ms, EventKind::Lagged { dt_ms }, hash);
        }
        if self.ended.is_none() && self.now_ms >= i64::from(self.match_cfg.max_duration_ms) {
            let reason = EndReason::Timeout;
            ledger_full |= !self.journal(self.now_ms, EventKind::MatchEnded { reason }, hash);
            self.ended = Some(MatchEnd {
                reason,
                t_ms: self.now_ms,
            });
        }
        // Doc 16 §4: at 100% of `ledger.max_bytes` the match is stopped with
        // `Timeout`. There is no event to append — the ledger is exactly what has
        // run out of room — so the end is recorded in the result and not in the
        // black box.
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
        StepOutput {
            ledger_chunks: self.ledger.take_chunks(),
            frames,
            tempo,
            next_due_ms: self.next_due_ms(wall_now_ms),
            lagged: self.clock.lagged() || self.ledger.lagged(),
            ended,
            substeps: ran,
            perception,
        }
    }

    /// **One substep** of doc 16 §3.1 phase 3, answering whether the ledger
    /// refused an event.
    ///
    /// (a) motion, in unit-id order, from the substep-start snapshot; (b) the index
    /// rebuilt; (c) arena time advanced and every due event at or before it
    /// applied. The events (a) and (c) produced are appended **last**, in that
    /// order — the `blocked` notices in unit-id order, then the due events in
    /// `(t_ms, seq)` order — against one hash taken at the substep's end.
    fn substep(&mut self, dt_ms: u32, perception: &mut PerceptionCounters) -> bool {
        let mut notices: Vec<Notice> = Vec::new();

        // ---- (a) motion ----
        {
            self.index.rebuild(&self.definition.terrain, &self.units);
            let terrain = &self.definition.terrain;
            let index = &mut self.index;
            for slot in 0..self.units.len() {
                let Some(unit) = self.units.get_mut(slot) else {
                    continue;
                };
                let id = unit.id;
                let was_blocked = unit.blocked;
                let from = terrain.cell_of(&unit.pose);
                let outcome = integrate(unit, dt_ms, terrain, index);
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
                // One notice per **stop**, not one per substep a standing order
                // holds the unit against the obstacle: the rising edge of
                // `blocked` is the event, and `blocked` itself is the condition.
                if let Some((x, y)) = outcome.blocked_at
                    && !was_blocked
                {
                    notices.push((0, EventKind::Blocked { unit: id, x, y }, None));
                }
            }
        }

        // ---- (b) the index over the positions motion committed ----
        self.index.rebuild(&self.definition.terrain, &self.units);

        // ---- (c) arena time, and every due event at or before it ----
        self.now_ms = self.now_ms.saturating_add(i64::from(dt_ms));
        for notice in &mut notices {
            // The `blocked` notices belong to the interval just integrated, which
            // ends here.
            notice.0 = self.now_ms;
        }
        while let Some(event) = self.due.pop_due(self.now_ms) {
            match event.kind {
                // Doc 16 §3.3 "Startup": `match.startup_ms` after `STARTUP` was
                // accepted. `resolve_startup` already journalled the transition
                // into `starting`, and doc 16 §4's kind list has no second event
                // for reaching `online`.
                DueKind::StartupComplete { unit } => {
                    if let Some(state) = self.unit_mut(unit)
                        && state.power == Power::Starting
                    {
                        state.power = Power::Online;
                    }
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
                            None,
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
                // A pass that leaves the unit holding anything classified schedules
                // a `Share` at the *same* timestamp: the heap orders
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
                    if self.unit(unit).is_some_and(|state| state.sensor.is_some()) {
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
                    self.share_pass(unit, event.t_ms, perception, &mut notices);
                }
                // Cut W schedules and applies the shot's flight time and the
                // weapon's cycle. Nothing before it schedules one, so nothing here
                // can fire — the arms exist so the later cut adds a body rather
                // than a shape.
                DueKind::Impact { .. } | DueKind::WeaponReady { .. } => {}
            }
        }

        // ---- the substep's record ----
        let hash = state_hash(self);
        self.substep_hashes.push(hash);
        let mut refused = false;
        for (t_ms, kind, draw) in notices {
            // 16b cut S.2: the pilot's notice and the ledger's event come off the
            // same journal entry, in the same order, so a line a pilot read is a
            // line the black box can be searched for.
            self.stage_pilot_notice(t_ms, &kind);
            refused |= !self.journal_drawn(t_ms, kind, draw, hash);
        }
        refused
    }

    /// The world asked the arena to stop (doc 16 §3.1's `Stop`): the reason is
    /// recorded, the match ends, and the ledger's tail leaves with it.
    ///
    /// The three reasons journal three different things, which is doc 16 §4's kind
    /// list read literally: `Abort` is an `aborted` event, `Timeout` is
    /// `match_ended{timeout}`, and `Void` is a `void` event — the one that says
    /// nothing the match derived may be committed.
    fn stopped(&mut self, wall_now_ms: i64, stop: StopReason) -> StepOutput {
        let hash = state_hash(self);
        let (kind, reason) = match stop {
            StopReason::Abort => (EventKind::Aborted, EndReason::Abort),
            StopReason::Timeout => (
                EventKind::MatchEnded {
                    reason: EndReason::Timeout,
                },
                EndReason::Timeout,
            ),
            StopReason::Void => (EventKind::Void, EndReason::Void),
        };
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
        }
    }

    /// Append one event, answering whether it was recorded. A `false` is the
    /// ledger's byte bound (doc 16 §4), which phase 5 turns into the end of the
    /// match; it is never an error the caller has to handle mid-substep, because a
    /// black box out of tape is not a reason to lose the pulse in progress.
    fn journal(&mut self, t_ms: i64, kind: EventKind, hash: u64) -> bool {
        self.ledger.append(t_ms, kind, None, hash).is_ok()
    }

    /// The same, with the draw doc 16 §4 asks an event to carry *where one
    /// occurred* — cut S.1's `classified` is the first event that has one.
    fn journal_drawn(
        &mut self,
        t_ms: i64,
        kind: EventKind,
        draw: Option<(String, u64)>,
        hash: u64,
    ) -> bool {
        self.ledger.append(t_ms, kind, draw, hash).is_ok()
    }

    /// One frame per pilot whose view changed, in pilot-id order (doc 16 §3.1's
    /// latest-wins slots).
    ///
    /// Two passes on purpose: the first reads the units, the second writes the
    /// remembered frames. A pilot seated on a unit this arena does not hold gets
    /// nothing rather than an empty frame.
    fn frames(&mut self, tempo: Tempo) -> Vec<(ObjId, Frame)> {
        self.sweep_held_losses();
        let mut candidates: Vec<(ObjId, Frame)> = Vec::with_capacity(self.pilots.len());
        for pilot in &self.pilots {
            let Some(unit) = self.unit(pilot.unit) else {
                continue;
            };
            let reach = unit
                .weapons
                .iter()
                .map(|weapon| weapon.max_range_cm)
                .max()
                .unwrap_or(0);
            candidates.push((
                pilot.pilot,
                Frame {
                    unit: unit.id,
                    team: unit.team,
                    pose: unit.pose.clone(),
                    speed_cms: unit.speed_cms,
                    power: unit.power,
                    blocked: unit.blocked,
                    tempo,
                    contacts: self.frame_contacts(unit.id),
                    weapon_reach_cm: reach,
                    // Per platform, so two pilots in one tank are both told.
                    notices: self
                        .pending_notices
                        .get(&unit.id)
                        .cloned()
                        .unwrap_or_default(),
                },
            ));
        }
        // One pulse's worth, and no more: a platform nobody is seated on
        // accumulates nothing.
        self.pending_notices.clear();
        candidates.sort_by_key(|(pilot, _)| *pilot);
        let mut changed: Vec<(ObjId, Frame)> = Vec::new();
        for (pilot, frame) in candidates {
            let told = !frame.notices.is_empty();
            if told
                || self
                    .last_frames
                    .get(&pilot)
                    .is_none_or(|last| !last.same_as(&frame))
            {
                // The remembered copy carries no notice: `CONTACTS` and `VIEW`
                // re-render it, and a notice said twice is a notice said wrong.
                let mut remembered = frame.clone();
                remembered.notices.clear();
                self.last_frames.insert(pilot, remembered);
                changed.push((pilot, frame));
            }
        }
        changed
    }

    /// **Stage the pilot notice one journalled perception event owes**, and apply
    /// [`FrameNotice`]'s hysteresis (16b cut S.2; doc 16 §3.6 "What a pilot sees").
    ///
    /// Called with every event the pass journalled, so the pilot's lines are a
    /// projection of the black box's own record and the two cannot disagree. Three
    /// arms and one rule each:
    ///
    /// * `detected` — an acquisition, **unless** a held loss for the same
    ///   `(platform, contact)` is still inside its window, in which case the
    ///   contact merely flickered: the held loss is cancelled and nothing is said.
    /// * `classified` — always said; being told apart is not something that
    ///   flickers (the pass keeps a classification across a gap by design).
    /// * `lost` — held for one further pass rather than said, which is the whole
    ///   hysteresis; [`Arena::sweep_held_losses`] is what eventually says it.
    fn stage_pilot_notice(&mut self, t_ms: i64, kind: &EventKind) {
        let window_ms = i64::from(self.sensors_cfg.pass_ms.max(1));
        match *kind {
            EventKind::Detected {
                viewer, contact, ..
            } => {
                if self.held_losses.remove(&(viewer, contact)).is_some() {
                    return;
                }
                self.stage_notice(
                    viewer,
                    FrameNotice {
                        kind: NoticeKind::Acquired,
                        contact,
                        class: Classification::Unknown,
                    },
                );
            }
            EventKind::Classified {
                viewer,
                contact,
                team,
                ..
            } => self.stage_notice(
                viewer,
                FrameNotice {
                    kind: NoticeKind::Classified,
                    contact,
                    class: Classification::Classified { team },
                },
            ),
            EventKind::Lost {
                viewer, contact, ..
            } => {
                self.held_losses
                    .insert((viewer, contact), t_ms.saturating_add(window_ms));
            }
            _ => {}
        }
    }

    /// Put one notice in the platform's queue for this pulse.
    fn stage_notice(&mut self, platform: ObjId, notice: FrameNotice) {
        self.pending_notices
            .entry(platform)
            .or_default()
            .push(notice);
    }

    /// **Say the losses whose hysteresis window has run out** (16b cut S.2), at the
    /// end of the pulse that reached it.
    ///
    /// A held loss whose platform has left the match is dropped rather than said:
    /// there is nobody it is a fact about any more.
    fn sweep_held_losses(&mut self) {
        let now = self.now_ms;
        let due: Vec<(ObjId, ContactId)> = self
            .held_losses
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|((viewer, contact), _)| (*viewer, *contact))
            .collect();
        for (viewer, contact) in due {
            self.held_losses.remove(&(viewer, contact));
            if self.unit(viewer).is_none() {
                continue;
            }
            self.stage_notice(
                viewer,
                FrameNotice {
                    kind: NoticeKind::Lost,
                    contact,
                    class: Classification::Unknown,
                },
            );
        }
    }

    /// **What one platform holds, as its pilots' frame carries it** (16b cut S.1;
    /// doc 16 §3.6 "What a pilot sees"), in contact-id order.
    ///
    /// Bearing and range are taken from the **target's current pose**, which is
    /// what a live sensor track is: the contact was refreshed within the last
    /// `sensors.pass_ms` or it would not be here, and doc 16 §3.6's observer rule is
    /// about *undetected* units — nothing here exists for a unit the platform does
    /// not hold. A contact whose target has left the match is dropped rather than
    /// reported at a stale bearing.
    #[must_use]
    pub fn frame_contacts(&self, platform: ObjId) -> Vec<FrameContact> {
        let Some(from) = self.unit(platform) else {
            return Vec::new();
        };
        self.contacts
            .of(platform)
            .filter_map(|(id, held)| {
                let target = self.unit(held.target)?;
                Some(FrameContact {
                    id: *id,
                    class: held.class,
                    bearing_mdeg: bearing_mdeg(&from.pose, &target.pose),
                    range_cm: range_cm(&from.pose, &target.pose),
                    age_ms: self.now_ms.saturating_sub(held.first_seen_ms),
                    cell: self.definition.terrain.cell_of(&target.pose),
                })
            })
            .collect()
    }

    /// When the task should step again, in wall milliseconds: the earlier of the
    /// next period boundary and the next due event (doc 16 §3.1 phase 5).
    ///
    /// The due event's time is arena time, so it is offered as the interval from
    /// *now* rather than as an absolute: an arena that is behind does not ask to be
    /// stepped in the past.
    fn next_due_ms(&self, wall_now_ms: i64) -> i64 {
        let period = i64::from(self.clock.tempo().period_ms(&self.arena_cfg.tempo));
        let by_period = wall_now_ms.saturating_add(period);
        match self.due.next_t_ms() {
            Some(due) => {
                let ahead = due.saturating_sub(self.now_ms).max(0);
                by_period.min(wall_now_ms.saturating_add(ahead))
            }
            None => by_period,
        }
    }
}

/// The remembered frames, as [`Arena`] holds them — a type alias so the field's
/// declaration reads as one thing rather than as a nested generic.
pub type FrameSlots = BTreeMap<ObjId, Frame>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::intents::{Control, ControlState};
    use crate::arena::ledger::LedgerEvent;
    use crate::arena::state::{Channel, Team};
    use crate::arena::test_support::{arena_at_epoch, pose, tank};
    use crate::config::{ArenaConfig, MatchConfig};

    const A: ObjId = ObjId(4);

    fn arena() -> Arena {
        arena_at_epoch(vec![tank(A, Team::Red, pose(3_500, 3_500))], 1)
    }

    fn startup() -> Intent {
        Intent {
            unit: A,
            channel: Channel::Helm,
            epoch: 1,
            kind: IntentKind::Startup,
            seq: 1,
        }
    }

    /// Everything the arena has journalled: the chunks it handed out as it ran,
    /// plus whatever is still open when the assertion asks.
    #[derive(Default)]
    struct Log {
        bytes: Vec<u8>,
    }

    impl Log {
        fn absorb(&mut self, out: &StepOutput) {
            for chunk in &out.ledger_chunks {
                self.bytes.extend_from_slice(&chunk.bytes);
            }
        }

        fn events(&mut self, arena: &mut Arena) -> Vec<LedgerEvent> {
            arena.ledger.flush();
            for chunk in arena.ledger.take_chunks() {
                self.bytes.extend_from_slice(&chunk.bytes);
            }
            String::from_utf8(self.bytes.clone())
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
                .collect()
        }
    }

    #[test]
    fn the_opening_pulse_establishes_the_wall_reference_and_integrates_nothing() {
        let mut arena = arena();
        let out = arena.step(1_000_000, &[], &[]);
        assert_eq!(out.substeps, 0);
        assert_eq!(arena.now_ms, 0);
        assert!(!out.lagged);
        assert!(out.ended.is_none());
        assert_eq!(out.tempo, Tempo::Quiet);
        // The next step is asked for at the period, or at the first summary,
        // whichever is sooner: both are 5,000 ms away here.
        assert_eq!(out.next_due_ms, 1_005_000);
        assert!(arena.substep_hashes.is_empty());
        // One frame, because the pilot has never been told anything.
        assert_eq!(out.frames.len(), 1);
        assert_eq!(
            out.frames.first().map(|(pilot, _)| *pilot),
            Some(ObjId(1000))
        );
        // And nothing more until something changes.
        let again = arena.step(1_000_000, &[], &[]);
        assert!(again.frames.is_empty());
    }

    #[test]
    fn a_pulse_runs_the_substeps_the_clamped_dt_is_cut_into() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        // 1,000 cm cells and a 1,000 cm/s tank: the substep is the 250 ms cap.
        let out = arena.step(1_000, &[], &[]);
        assert_eq!(out.substeps, 4);
        assert_eq!(arena.now_ms, 1_000);
        assert_eq!(arena.substep_hashes.len(), 4);
        // Every substep's hash is recorded, and they differ because `now_ms` does.
        let unique: std::collections::BTreeSet<u64> =
            arena.substep_hashes.iter().copied().collect();
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn startup_is_queued_in_phase_two_and_resolved_in_phase_four() {
        let mut arena = arena();
        let out = arena.step(0, &[], &[startup()]);
        assert!(arena.pending.is_empty(), "phase 4 drained the queue");
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Starting));
        assert!(out.ended.is_none());
        // `match.startup_ms` later, at the substep boundary at or after it.
        let mut wall = 0_i64;
        for _ in 0..3 {
            wall += 5_000;
            arena.step(wall, &[], &[]);
        }
        assert_eq!(arena.now_ms, 15_000);
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Online));
    }

    #[test]
    fn a_stop_ends_the_match_at_phase_one_and_a_further_step_does_nothing() {
        for (stop, reason, word) in [
            (StopReason::Abort, EndReason::Abort, "aborted"),
            (StopReason::Timeout, EndReason::Timeout, "match_ended"),
            (StopReason::Void, EndReason::Void, "void"),
        ] {
            let mut arena = arena();
            let mut journal = Log::default();
            journal.absorb(&arena.step(0, &[], &[]));
            arena.stop(stop);
            let out = arena.step(5_000, &[], &[]);
            journal.absorb(&out);
            assert_eq!(out.ended.map(|end| end.reason), Some(reason));
            assert_eq!(out.substeps, 0);
            assert_eq!(arena.now_ms, 0, "a stop never advances arena time");
            let log = journal.events(&mut arena);
            assert_eq!(log.last().map(|event| event.kind.as_str()), Some(word));
            // And the arena is finished with.
            let after = arena.step(10_000, &[], &[]);
            assert_eq!(after.ended.map(|end| end.reason), Some(reason));
            assert!(after.ledger_chunks.is_empty());
            assert!(after.frames.is_empty());
            assert_eq!(after.substeps, 0);
        }
    }

    #[test]
    fn the_duration_bound_ends_the_match_with_a_timeout() {
        let mut arena = Arena::new(
            crate::arena::test_support::custody(
                vec![tank(A, Team::Red, pose(3_500, 3_500))],
                Vec::new(),
            ),
            &ArenaConfig::default(),
            &MatchConfig {
                max_duration_ms: 3_000,
                ..MatchConfig::default()
            },
            &crate::config::SensorsConfig::default(),
        )
        .expect("a three-second match");
        let mut journal = Log::default();
        journal.absorb(&arena.step(0, &[], &[]));
        let out = arena.step(2_000, &[], &[]);
        journal.absorb(&out);
        assert!(out.ended.is_none());
        let out = arena.step(4_000, &[], &[]);
        journal.absorb(&out);
        assert_eq!(
            out.ended,
            Some(MatchEnd {
                reason: EndReason::Timeout,
                t_ms: 4_000
            })
        );
        let log = journal.events(&mut arena);
        assert_eq!(
            log.last().map(|event| event.kind.as_str()),
            Some("match_ended")
        );
        // And a further step does nothing at all.
        let after = arena.step(6_000, &[], &[]);
        assert_eq!(after.substeps, 0);
        assert!(after.ledger_chunks.is_empty());
        assert_eq!(arena.now_ms, 4_000);
    }

    #[test]
    fn a_released_seat_in_the_same_burst_purges_the_order_it_arrived_with() {
        // Doc 16 §3.1's burst, end to end through `step`: phase 1 raises the
        // epoch, phase 2 refuses the order that was typed under the old one.
        let mut arena = arena();
        let out = arena.step(
            0,
            &[Control {
                unit: A,
                channel: Channel::Helm,
                epoch: 2,
                state: ControlState::Released,
            }],
            &[startup()],
        );
        assert!(out.ended.is_none());
        assert_eq!(arena.unit(A).map(|unit| unit.power), Some(Power::Off));
        assert!(arena.pending.is_empty());
        assert_eq!(arena.due.len(), 1, "only the position summary is scheduled");
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert_eq!(
            log.iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["match_started"]
        );
    }

    #[test]
    fn a_fire_intent_raises_the_alert_tempo_and_journals_the_change() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        let fire = Intent {
            unit: A,
            channel: Channel::Fire,
            epoch: 1,
            kind: IntentKind::Fire {
                weapon: smol_str::SmolStr::new("cannon"),
            },
            seq: 2,
        };
        let out = arena.step(5_000, &[], &[fire]);
        // A queued fire intent is an `Alert` trigger, not an `Engaged` one (doc 16
        // §3.1's table): the shot itself is cut W's.
        assert_eq!(out.tempo, Tempo::Alert);
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter().any(|event| event.kind
                == EventKind::Tempo {
                    from: Tempo::Quiet,
                    to: Tempo::Alert
                }),
            "the tempo change was not journalled: {log:?}"
        );
    }

    #[test]
    fn the_position_summary_keeps_its_cadence_in_arena_time() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        let mut wall = 0_i64;
        for _ in 0..4 {
            wall += 5_000;
            arena.step(wall, &[], &[]);
        }
        assert_eq!(arena.now_ms, 20_000);
        let mut journal = Log::default();
        let log = journal.events(&mut arena);
        let summaries: Vec<i64> = log
            .iter()
            .filter(|event| event.kind.as_str() == "position_summary")
            .map(|event| event.t_ms)
            .collect();
        assert_eq!(summaries, vec![5_000, 10_000, 15_000, 20_000]);
        // The next one is scheduled, always.
        assert_eq!(arena.due.next_t_ms(), Some(25_000));
    }

    /// **16b cut S.1, the pulse end to end**: the sensor pass is due in phase
    /// 3(c), the contact it mints reaches the pilot's frame, the tempo answers to
    /// it, and the counters say what it cost.
    #[test]
    fn a_sensor_pass_runs_in_the_pulse_and_its_contact_reaches_the_frame_and_the_tempo() {
        use crate::arena::test_support::scout;
        const B: ObjId = ObjId(9);
        // Two tanks that see and shoot, four cells apart on the 8 by 8 window. The
        // shipped cannon reaches 2,500 m, so the derived `tempo.alert_range_cm` is
        // 5,000 m and a contact 40 m away is well inside it.
        let mut arena = crate::arena::test_support::arena_at_epoch(
            vec![
                scout(A, Team::Red, pose(500, 500)),
                scout(B, Team::Blue, pose(4_500, 500)),
            ],
            1,
        );
        assert_eq!(arena.alert_range_cm, 500_000, "twice the cannon's reach");
        // The opening pulse establishes the wall reference and integrates nothing,
        // so no pass has run yet.
        let out = arena.step(0, &[], &[]);
        assert_eq!(out.perception.passes, 0);
        assert!(arena.contacts.is_empty());
        // One second of arena time: both staggered passes (at 4 ms and 9 ms) run,
        // each holds the other, and the tempo rises to `Alert`.
        let out = arena.step(1_000, &[], &[]);
        assert_eq!(out.perception.passes, 2, "{:?}", out.perception);
        assert_eq!(out.perception.detected, 2);
        assert_eq!(out.perception.traces, 2);
        assert!(out.perception.cell_probes >= 8);
        assert_eq!(out.tempo, Tempo::Alert);
        assert_eq!(arena.contacts.len(), 2);
        // The pilot of A is told about B, at the right bearing and range, and about
        // nothing else.
        let frame = out
            .frames
            .iter()
            .find(|(pilot, _)| *pilot == ObjId(1000))
            .map(|(_, frame)| frame)
            .unwrap_or_else(|| panic!("A's pilot was sent a frame: {:?}", out.frames));
        assert_eq!(frame.contacts.len(), 1);
        let contact = frame
            .contacts
            .first()
            .copied()
            .unwrap_or_else(|| panic!("one contact"));
        assert_eq!(contact.class, crate::arena::state::Classification::Unknown);
        assert_eq!(contact.bearing_mdeg, 90_000, "B is due east of A");
        assert_eq!(contact.range_cm, 4_000);
        // And it is the id `draw_contact_id` mints for the pair, so a pilot's
        // `LOCK` resolves through `ShadowContacts::resolve`.
        assert_eq!(
            contact.id,
            crate::arena::rng::draw_contact_id(arena.seed, A, B)
        );
        assert!(arena.contacts.resolve(A, contact.id).is_some());
        assert!(
            arena.contacts.resolve(B, contact.id).is_none(),
            "an id minted for A is not B's"
        );
        // Three more seconds carry both past `classify_after_ms`, and the classify
        // draw is in the ledger against the label doc 16 §3.5 pins.
        let mut journal = Log::default();
        let mut wall = 1_000_i64;
        for _ in 0..4 {
            wall += 1_000;
            journal.absorb(&arena.step(wall, &[], &[]));
        }
        let log = journal.events(&mut arena);
        let classified: Vec<&LedgerEvent> = log
            .iter()
            .filter(|event| event.kind.as_str() == "classified")
            .collect();
        assert_eq!(classified.len(), 2, "{:?}", log);
        for event in classified {
            let (label, _) = event
                .draw
                .clone()
                .unwrap_or_else(|| panic!("a classification carries its draw: {event:?}"));
            assert!(label.starts_with("classify:"), "{label}");
        }
    }

    #[test]
    fn a_pulse_past_the_clamp_journals_one_lagged_notice() {
        let mut arena = arena();
        arena.step(0, &[], &[]);
        // Quiet is a 5,000 ms period and `max_catch_up_steps` is 4, so 20,000 ms
        // is the clamp and 20,001 is lag.
        let out = arena.step(30_000, &[], &[]);
        assert!(out.lagged);
        assert_eq!(arena.now_ms, 20_000, "the idle wall time is skipped");
        let mut journal = Log::default();
        journal.absorb(&out);
        let log = journal.events(&mut arena);
        assert!(
            log.iter()
                .any(|event| event.kind == EventKind::Lagged { dt_ms: 20_000 })
        );
    }
}
