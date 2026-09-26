//! **One running match, as a task** (16b cut T.2, T1; doc 16 §3.1 "The seam is
//! eight messages", "The arena core is a library", "Clock and lag", "Failure").
//!
//! [`crate::arena`] is a library with no clock, no task and no I/O. This is the
//! thin wrapper doc 16 §3.1 describes and the *only* place the arena meets a
//! runtime: it owns one [`crate::arena::Arena`], a bounded inbox, a bounded egress, a timer
//! driven by [`crate::arena::StepOutput::next_due_ms`], a watchdog at `tempo.stall_ms`, the
//! unwind boundary, and a `watch` stop signal. It is deliberately **not** under
//! `arena/`: the gate in `crates/mush-contract-tests/tests/arena_library.rs`
//! refuses the word `tokio` in every file of that directory, and this file is the
//! reason that gate can be strict.
//!
//! # What crosses, and in which direction
//!
//! ```text
//! world -> task   ArenaMsg::Intent      one order, stamped with the world's epoch and seq
//! world -> task   ArenaMsg::Control     this (unit, channel) is now at this epoch
//! world -> task   ArenaMsg::Stop        abort, timeout or void
//! world -> task   ArenaMsg::LedgerAck   every chunk up to this sequence is durable
//! world -> task   ArenaMsg::TempoFloor  staff floor the wake interval, or lift it (I.3)
//! world -> task   ArenaMsg::Watch       somebody is watching, or nobody is any more (I.4)
//! task  -> world  ArenaOut::Ledger      one closed chunk of the match's black box
//! task  -> world  ArenaOut::Frames      the pilots whose view changed, the arena's clock,
//!                                       and the staff picture while watched (I.4)
//! task  -> world  ArenaOut::Lagged      the clock or the egress is behind
//! task  -> world  ArenaOut::Deliberate  the squads with something new to decide (N.2)
//! task  -> world  ArenaOut::Ended       the match is over, with its ledger digest
//! ```
//!
//! `Deliberate` is doc 16 §3.1's eighth conceptual seam message. The table
//! includes `LedgerAck` and `Lagged` reliability signals, **16b cut I.3**'s
//! `TempoFloor` and **16b cut I.4**'s `Watch`, staff bookkeeping applied on
//! arrival, while custody out is a spawn argument: it lists all eleven runtime
//! enum variants.
//! [`crate::arena_task::ArenaOut::Deliberate`] explains its report-only contract.
//!
//! `CustodyOut` is the argument to [`crate::arena_task::spawn_arena`] rather than a message — a
//! match is checked out exactly once, at the moment it starts — and `CustodyIn`
//! is cut T.3's: this cut ends a match with [`crate::arena_task::ArenaOut::Ended`] and T.3 turns
//! that into a check-in.
//!
//! # The timer, and why a message never steps
//!
//! The task sleeps until `next_due_ms` (doc 16 §3.1 phase 5, as 16b cut U.1 made
//! it: the earlier of the next multiple of the period and the first substep
//! boundary at or after the next due event, in absolute arena time, the deadline
//! measured from the task's base). An arriving message **wakes the
//! task early but does not step it**: it is buffered and drained at the next
//! pulse (§3.1 phases 1 and 2). That is what makes a laggy pilot harmless and a
//! fast one no faster — the tempo decides how often `step` runs, and nothing
//! else does.
//!
//! **Two messages pull the deadline in** (16b cut I.3): a `Stop` and a
//! `TempoFloor` set the next wake to a millisecond from now, since the loop
//! otherwise recomputes its deadline only after a pulse — under a 10,000 ms
//! floor an abort, a set or a lift would wait out the old wake. The pulled-in
//! pulse is an ordinary pulse: a stop is honoured at its phase 1 before any time
//! moves, and a floored pulse is quantised to the substep lattice by
//! `Arena::step`, so it is rate-independent by construction. The immediate lever
//! is still the stop signal, which ends the task with no result.
//!
//! # What waits for the pulse (16b cut M.1, `B-50`, `T-11`)
//!
//! Between two pulses the task holds four buffers, each with its own rule, and
//! **nothing is dropped to make room for anything else**:
//!
//! - **controls**, one per `(unit, channel)`, the highest epoch standing — at most
//!   `units × 3` of them, so they need no bound — beside the **releases** a later
//!   control displaced, one per `(unit, channel, pilot)`, which run first so a
//!   pilot who left is taken off the frame roster before the seat's next holder
//!   raises the table (`buffer_control`);
//! - **continuous orders**, one per `(unit, kind)`, the highest `seq` standing —
//!   phase 2 applies them latest-wins anyway (`buffer_continuous`);
//! - **discrete orders**, per unit, bounded **per unit** at
//!   `intents.max_pending_per_unit + 1`, the newest arrival refused when that
//!   unit's buffer is full (`buffer_discrete`) — so one pilot's pasted burst
//!   can neither drop an earlier `LOCK` of their own while its `FIRE` survives
//!   nor drop another pilot's order at all. **16b cut I.3**: a staff `Place` is
//!   the exception — kept past the bound and counted in nobody's.
//!
//! The inbox itself keeps `reserve` slots an intent (and, 16b cut I.3, a
//! `TempoFloor`; 16b cut I.4, a `Watch`) may never take, so a `Control`, a
//! `Stop` or a `LedgerAck` always fits
//! ([`crate::arena_task::ArenaHandle::offer`]).
//!
//! # Failure
//!
//! `step` runs under [`std::panic::catch_unwind`]. A panic ends the task with
//! [`crate::arena_task::ArenaOut::Ended`] carrying [`crate::arena::EndReason::Void`], and **nothing the faulted
//! pulse produced leaves** — there is no `StepOutput` to publish, and the task
//! never touches the arena's chunk egress again. A pulse that takes longer than
//! `tempo.stall_ms` is voided by the watchdog after it returns (the arena has no
//! preemption, so a stalled pulse cannot be interrupted, only refused a
//! successor — or, when the stalled pulse is the one that ended the match,
//! refused its result: the match is void at the task, no unit is returned, and
//! the black box keeps whatever end the arena journalled — `match_ended`, or
//! `aborted` for an abort — and no `void` event; 16b cut AA.3). Both are
//! consistent with the world task's own firewall: nothing a faulted turn touched
//! reaches the store.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mush_core::ObjId;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::arena::damage::UnitReturn;
use crate::arena::intents::{Control, ControlState, Intent, IntentKind};
use crate::arena::ledger::LedgerChunk;
use crate::arena::observer::Observer;
use crate::arena::state::{
    Arena, ArenaError, Channel, CustodyOut, Designator, EndReason, MatchEnd, StopReason,
};
use crate::arena::step::{Frame, StepOutput};
use crate::config::{ArenaConfig, MatchConfig, SensorsConfig};

/// **One match's identity**, allocated by the world half.
///
/// Cut T.3 owns the counter that mints these; this cut only carries the value,
/// stamps every egress message with it, and names the match's ledger log after
/// it. It is deliberately a `u64` and not a store id: an OOC match holds no
/// durable record of its own while it runs (D-16-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatchId(pub u64);

impl std::fmt::Display for MatchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// **World to arena** (doc 16 §3.1's first four messages).
#[derive(Clone, Debug)]
pub enum ArenaMsg {
    /// One order, already validated at the world (focus, lease, channel,
    /// epoch) and authorised again at apply inside the arena.
    Intent(Intent),
    /// This `(unit, channel)` is now at this epoch, and the seat is in this
    /// state.
    Control(Control),
    /// Stop the match. Honoured at the next pulse's phase 1, never mid-substep.
    Stop(StopReason),
    /// Every chunk up to and including `upto_chunk_seq` has landed in the
    /// world's `log:match.<id>`. This is the whole of the egress's flow control
    /// (doc 16 §3.1: the egress is bounded and the arena voids the match past
    /// `ledger.max_unflushed_chunks`).
    LedgerAck {
        /// The highest durable chunk sequence.
        upto_chunk_seq: u64,
    },
    /// **Set or lift the staff tempo floor** (16b cut I.3; 16c N10:
    /// `@arena/tempo`): `Some(ms)` floors the wake interval at `ms` of wall
    /// time, `None` lifts it. Seam bookkeeping and not an input to `step`: it is
    /// applied on arrival (`Clock::set_floor`), as a `LedgerAck` is, and it pulls
    /// the task's deadline in to a millisecond away so the set or the lift takes
    /// effect at once rather than after the old wake. The bounds are the verb's.
    TempoFloor {
        /// The floor, milliseconds; `None` lifts it.
        period_ms: Option<u32>,
    },
    /// **Somebody is watching this match, or nobody is any more** (16b cut
    /// I.4; 16c N10: `@arena/watch`). Seam bookkeeping and not an input to
    /// `step`, applied on arrival as a `TempoFloor` is (`apply_watch`): it
    /// sets `Arena::watched`, and a `true` clears `Arena::last_observer_s` so
    /// the next pulse builds a picture for the new watcher at once. Held to
    /// the intent reserve at [`ArenaHandle::offer`]. The world sends `true` for
    /// a match's first watcher and `false` when its last one goes; who the
    /// watchers are is the world's and never crosses.
    Watch {
        /// Whether anybody is watching.
        on: bool,
    },
}

/// **Arena to world** (doc 16 §3.1's last three messages, plus the end).
#[derive(Clone, Debug)]
pub enum ArenaOut {
    /// One closed chunk of the match ledger, to be appended to
    /// `log:match.<id>` on the world's own writer lane.
    Ledger {
        /// Which match.
        match_id: MatchId,
        /// The chunk.
        chunk: LedgerChunk,
    },
    /// The pilots whose view changed this pulse, in pilot-id order, **and the
    /// arena's own clock**.
    ///
    /// **16b cut M.1 (`B-77`): a frame the world could not take is held at the
    /// task and merged, never lost.** `Arena::frames` clears the pulse's staged
    /// notices and commits the view when it builds the frame, so a `Frames` the
    /// full egress refused used to take every notice of that pulse — for every
    /// pilot of the match — with it. Now `Task::publish` keeps the refused
    /// message and merges it into the next pulse's: per pilot the **view is
    /// latest-wins** (the newer frame's) and the **notices are kept in order**
    /// (the held ones first), at most sixty-four of a pilot's held ones
    /// surviving each merge (a frame nothing was held for is never cut); a
    /// pilot who has been
    /// released since is dropped from it; and the last held message **waits at
    /// the end** — `Task::end` sends it before [`ArenaOut::Ended`]. A notice is
    /// delivered at most once, and at least once unless the process is stopping.
    ///
    /// **16b cut D.1 put the clock on it and made the message sendable empty.**
    /// The board's `arena time` line had nothing per-pulse to mirror from and read
    /// the last closed ledger chunk's timestamp instead, which is nought for the
    /// first minutes of a match and a minute stale after that. There is no
    /// per-pulse frame on a match nobody's view is changing in, so the message
    /// itself carries the clock and `Task::publish` sends it when there is a
    /// frame **or** when the arena second has moved since the last one it sent.
    ///
    /// **What the rule does, stated against the wake rate and not against the
    /// tempo.** A pulse is not a period: the task sleeps until `next_due_ms`,
    /// which is *the earlier of the next multiple of the period and the first
    /// substep boundary at or after the next due event*, both in absolute arena
    /// time (16b cut U.1; see this module's head), so every wake is on the
    /// substep lattice (250 ms on the shipped content) and a match wakes at most
    /// once a substep. On the shipped config the recurring due events are the
    /// per-unit sensor pass (`sensors.pass_ms`, 1,000 ms) and the reflex decision
    /// (`reflex.decide_ms`, 1,000 ms), each at the unit's own seeded offset inside
    /// the second (`crate::arena::rng::stagger_ms`): a field of many units wakes
    /// on every lattice point, and two tanks on the one to four points their
    /// offsets round up to.
    ///
    /// The clause therefore suppresses an empty message whenever a pulse lands in
    /// a second the last message already carried, so the steady state is **at
    /// most one empty message per match per arena second**. The board is stale by
    /// the gap between the first wakes of two arena seconds. At `Engaged` and
    /// `Alert` every second opens with a wake (a multiple of the period), so the
    /// gap is one arena second. At `Quiet` only every fifth second does; any other
    /// second opens at its earliest recurring offset rounded up to the lattice,
    /// so the gap after a `Quiet` period multiple is one second **plus that
    /// offset** — at most `2,000 − substep` ms, 1,750 on the shipped lattice, and
    /// one second only where some unit's offset rounds up to the second's start.
    /// A config that pushes the sensor pass past the tempo (the `arena_task`
    /// tests do, to see the clause at all) gets one message a pulse at `Alert`
    /// and `Quiet` and one every other pulse at `Engaged`, and a board one period
    /// stale.
    ///
    /// **The new steady-state traffic is one small droppable message per match
    /// per arena second.** A match whose pilots are all parked sent nothing
    /// before this and now sends an empty `Frames`. It is still `try_send`, and an
    /// **empty** one the world could not take is still dropped: a clock the world
    /// missed is superseded by the next pulse's. Only a message carrying a frame
    /// is held (16b cut M.1, above).
    Frames {
        /// Which match.
        match_id: MatchId,
        /// One frame per pilot whose view changed. **May be empty**, when the
        /// message is carrying nothing but the clock.
        frames: Vec<(ObjId, Frame)>,
        /// **The arena's own `now_ms` after the step that produced this
        /// message** (16b cut D.1). Monotonic within a match.
        now_ms: i64,
        /// **The staff picture** (16b cut I.4), when the pulse built one — at
        /// most once per pulse and at most once an arena second, while the
        /// match is watched ([`crate::arena::step::StepOutput::observer`]).
        /// A message that carries one is sent even with no frame and inside
        /// a second the clock already carried; one the full egress refused is
        /// held, the newer picture replacing the older (`Task::held_observer`),
        /// and sent by `Task::end` with the held frames. The world publishes it
        /// on `arena.observer` to the match's watchers and to no pilot.
        observer: Option<Observer>,
    },
    /// The clock absorbed more wall time than the clamp allows, or the egress
    /// is backing up past half of `ledger.max_unflushed_chunks`.
    Lagged {
        /// Which match.
        match_id: MatchId,
    },
    /// **The squads with something new to decide about** (16b cut N.2; doc 16 §6
    /// "Deliberation").
    ///
    /// **This is the eighth conceptual message in doc 16 §3.1.** It is
    /// here because doc 16 §6 puts the deliberation trigger on *the commander's
    /// contact set* — which only the arena can see — and the request itself on the
    /// world's own provider path, so something has to cross. It is the narrowest
    /// thing that can: **egress-only, report-only, and droppable**. It carries no
    /// authority, asks for nothing back on its own account (the reply is an
    /// ordinary [`ArenaMsg::Intent`]), and is sent with the same `try_send` frames
    /// are, because a signal the world misses is re-sent by the next pulse whose
    /// digest still differs. Nothing in the arena waits for it or reads a
    /// consequence of it.
    Deliberate {
        /// Which match.
        match_id: MatchId,
        /// The squads, in squad-id order, already floored by
        /// `[tacsim.arena].deliberate_min_ms` in arena time.
        signals: Vec<crate::arena::command::SquadSignal>,
    },
    /// The match is over. The last chunks precede this message.
    Ended {
        /// Which match.
        match_id: MatchId,
        /// Why, and when in arena time.
        end: MatchEnd,
        /// The SHA-256 of every ledger byte the match streamed, for the result
        /// record (doc 16 §4) — **the bytes that left in chunks**, in chunk order
        /// (16b cut M.1, `B-48`: `Ledger::shipped_digest`).
        ///
        /// The ending step calls `Ledger::flush` and then `Ledger::take_chunks`,
        /// so on every end whose step ran to completion — an abort, an
        /// elimination, a timeout, a spent ledger, and the AA.3 stall-forced void
        /// after a normal end — this is equal to `Ledger::digest`, the digest of
        /// every byte appended. On the panic arm the faulted step never flushed,
        /// and the open chunk never leaves; there the digest over what left the
        /// ledger is the only claim the world's `log:match.<id>` can honour, and
        /// `Task::end` waits for every queued chunk to reach the world before it
        /// sends this, so a world that wrote every chunk it was given reproduces
        /// it from the log.
        digest: [u8; 32],
        /// **Every unit, as the check-in takes it back** (16b cut W.2): its cells,
        /// its final pose, what became of it and its two counters — doc 16 §3.1's
        /// `CustodyIn { units[{cells, final_pose, outcome}] }`, which cut T.2 left
        /// empty because there was no damage model to fill it.
        ///
        /// It rides the **end** and not a message of its own: check-in happens once,
        /// at the end, and a unit whose cells arrived separately would be a unit the
        /// world could commit before it knew whether the match was void.
        units: Vec<UnitReturn>,
    },
}

impl ArenaOut {
    /// Which match this message is about.
    #[must_use]
    pub const fn match_id(&self) -> MatchId {
        match self {
            ArenaOut::Ledger { match_id, .. }
            | ArenaOut::Frames { match_id, .. }
            | ArenaOut::Lagged { match_id }
            | ArenaOut::Deliberate { match_id, .. }
            | ArenaOut::Ended { match_id, .. } => *match_id,
        }
    }
}

/// **The world's end of one running match.**
///
/// Cloneable it is not: the stop signal is a `watch::Sender`, and one match has
/// one owner. Cut T.3's registry holds it; the process's shutdown tail reaches
/// it through the same shared table to set every stop signal before joining.
#[derive(Debug)]
pub struct ArenaHandle {
    /// Which match.
    match_id: MatchId,
    /// The bounded inbox, `intents.max_pending_per_unit × units + reserve`
    /// messages. Its bound is the world side's own — the arena's per-seat rate
    /// (`intents.max_pending_per_unit` discrete orders per arena second) applies
    /// again at apply, so a pilot who floods this queue still cannot order more
    /// than that.
    inbox: mpsc::Sender<ArenaMsg>,
    /// **The inbox slots an intent may never take** (16b cut M.1, `B-50`):
    /// `3 × units + 2` — one control per channel per unit, a stop, and an ack.
    ///
    /// The inbox is one channel shared by intents, controls, stops and acks, so
    /// a burst of orders that filled it used to refuse the `EXIT` behind it: the
    /// world had already raised its epoch, the arena never heard, and the tank
    /// drove on the departed pilot's speed while every later order on that
    /// channel was refused `epoch`. [`ArenaHandle::offer`] keeps these free.
    reserve: usize,
    /// The shutdown tail's lever: `true` ends the task without an
    /// [`ArenaOut::Ended`], because a process that is stopping is not a match
    /// that has a result.
    stop: watch::Sender<bool>,
    /// **Whether the task has handed the world its [`ArenaOut::Ended`]** (16b cut
    /// M.1, `B-49`): set by `Task::end` the moment that send completes, and never
    /// cleared. Read by [`ArenaHandle::has_reported`].
    reported: Arc<AtomicBool>,
}

impl ArenaHandle {
    /// Which match this handle drives.
    #[must_use]
    pub const fn match_id(&self) -> MatchId {
        self.match_id
    }

    /// Offer one message to the arena **without waiting**.
    ///
    /// The world task is the single writer for everything outside the arena and
    /// must never block on it, so a full inbox is a refusal the caller renders
    /// and not a stall (doc 16 §3.7's refusals). `Err(msg)` hands the message
    /// back: the inbox is full, or the task has ended.
    ///
    /// **16b cut M.1** (`B-50`): an [`ArenaMsg::Intent`] is also handed back
    /// when the inbox's free room is down to its reserve (`3 × units + 2`
    /// slots, one control per channel per unit, a stop and an ack) — exactly as a
    /// full channel hands it back, so `forward_intent` still answers `Busy`
    /// through the same `Err`. A `Control`, a `Stop` and a `LedgerAck` may use
    /// the whole channel, so an `EXIT` behind a pasted burst of orders always
    /// fits.
    ///
    /// **16b cut I.3**: an [`ArenaMsg::TempoFloor`] is held to the same rule as
    /// an intent. The reserve is sized for one control per channel per unit, a
    /// stop and an ack, and a staffer repeating `@arena/tempo` must not be able
    /// to take the slot an `EXIT` needs; a refused floor is the verb's `busy`.
    /// **16b cut I.4**: and so is an [`ArenaMsg::Watch`], for the same reason.
    pub fn offer(&self, msg: ArenaMsg) -> Result<(), ArenaMsg> {
        if matches!(
            msg,
            ArenaMsg::Intent(_) | ArenaMsg::TempoFloor { .. } | ArenaMsg::Watch { .. }
        ) && self.inbox.capacity() <= self.reserve
        {
            return Err(msg);
        }
        self.inbox.try_send(msg).map_err(|error| match error {
            TrySendError::Full(msg) | TrySendError::Closed(msg) => msg,
        })
    }

    /// Whether the task is still accepting messages.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.inbox.is_closed()
    }

    /// **Whether the task has handed the world its [`ArenaOut::Ended`]** (16b cut
    /// M.1, `B-49`).
    ///
    /// `true` from the moment that send completed: the result is in the shared
    /// egress, or already staged for the world's drain, and the world's own
    /// `on_ended` will finish the match with the arena's units. A task whose
    /// inbox is closed and which has **not** reported died without a result —
    /// the stop signal, or a dropped inbox.
    #[must_use]
    pub fn has_reported(&self) -> bool {
        self.reported.load(Ordering::Acquire)
    }

    /// Ask the task to end **now**, with no result and no further egress — the
    /// shutdown tail's one lever. Idempotent.
    pub fn signal_stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// **Every unit's callsign, in unit-id order** (16b cut V.2): what
/// [`Arena::callsigns`] answers and [`spawn_arena`] hands back beside the task.
pub type Callsigns = Vec<(ObjId, Designator)>;

/// **Start one match** (16b T1).
///
/// The arena is built first, on the caller's thread, so a refused check-out
/// ([`ArenaError`]) is the caller's error and not a task that dies on its first
/// poll. Only once it holds an arena does this spawn anything.
///
/// The inbox bound is `intents.max_pending_per_unit × units + reserve(units)` —
/// the world side's own bound *before* the arena's, so a burst is refused at the
/// seam rather than filling a queue the arena would then have to purge, plus the
/// `3 × units + 2` slots only a control, a stop or an ack may take (16b cut M.1;
/// [`ArenaHandle::offer`]). The egress `Sender` is shared by every match on the
/// world (one receiver, drained by the world loop).
///
/// **16b cut V.2**: the third element of the answer is every unit's callsign
/// ([`Arena::callsigns`]), read off the arena this built **before** it moves into
/// the task — the slot rule of 16b cut Q.1 is applied once, by `Arena::new`, and
/// the result record names each unit by what that one application said. Nothing
/// is computed for it: it is a read of state the constructor already fixed.
///
/// # Errors
///
/// Whatever [`Arena::new`] refuses: a unit off the window, two units in one
/// cell, a duplicate id, a pilot seated on a unit that was not handed over.
pub fn spawn_arena(
    runtime: &tokio::runtime::Handle,
    match_id: MatchId,
    custody: CustodyOut,
    arena_cfg: &ArenaConfig,
    match_cfg: &MatchConfig,
    sensors_cfg: &SensorsConfig,
    egress: mpsc::Sender<ArenaOut>,
) -> Result<(ArenaHandle, JoinHandle<()>, Callsigns), ArenaError> {
    let units = custody.units.len();
    let arena = Arena::new(custody, arena_cfg, match_cfg, sensors_cfg)?;
    // 16b cut V.2: read before `arena` is moved into the task below.
    let callsigns = arena.callsigns();
    let per_unit = usize::try_from(arena_cfg.intents.max_pending_per_unit).unwrap_or(usize::MAX);
    let reserve = reserve(units);
    // Clamped to what tokio's channel accepts, which is a bound no real config
    // comes near; `max(1)` because a zero-capacity channel is not a channel.
    let capacity = per_unit
        .saturating_mul(units)
        .saturating_add(reserve)
        .clamp(1, tokio::sync::Semaphore::MAX_PERMITS);
    let (inbox_tx, inbox_rx) = mpsc::channel(capacity);
    let (stop_tx, stop_rx) = watch::channel(false);
    // **16b cut M.1** (`T-11`): the discrete buffer's bound is per unit, and one
    // above the rate — the extra one is what lets the arena itself say
    // `queue_full` for the first order past a seat's rate, so a pilot who
    // overruns is told once. Without a bound a pilot who kept the inbox full for
    // a whole 5,000 ms `Quiet` period would grow the task's memory without
    // limit: the inbox bounds what is *in flight*, this bounds what is *waiting
    // for the pulse*.
    let per_unit_bound = per_unit.saturating_add(1);
    let backlog_bound = usize::try_from(arena_cfg.ledger.max_unflushed_chunks)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    let stall = Duration::from_millis(u64::from(arena_cfg.tempo.stall_ms));
    let reported = Arc::new(AtomicBool::new(false));
    let join = runtime.spawn(arena_loop(Task {
        match_id,
        arena,
        inbox: inbox_rx,
        egress,
        stop: stop_rx,
        backlog: VecDeque::new(),
        backlog_bound,
        per_unit_bound,
        stall,
        last_clock_s: None,
        held_frames: None,
        held_observer: None,
        held_notice_drops: 0,
        reported: Arc::clone(&reported),
    }));
    Ok((
        ArenaHandle {
            match_id,
            inbox: inbox_tx,
            reserve,
            stop: stop_tx,
            reported,
        },
        join,
        callsigns,
    ))
}

/// **The inbox slots an intent may never take** (16b cut M.1, `B-50`): one
/// control per channel per unit, a stop, and an ack — `3 × units + 2`.
const fn reserve(units: usize) -> usize {
    units.saturating_mul(3).saturating_add(2)
}

/// **At most this many of one pilot's held notices survive a merge** (16b cut
/// M.1, `B-77`): while the world cannot take a frame, the oldest of a pilot's
/// held and new notices go past it, counted in `Task::held_notice_drops` and
/// warned of once a match. A frame nothing was held for is never cut.
const MAX_HELD_NOTICES: usize = 64;

/// Everything one match's task owns. A struct rather than eight arguments
/// because the loop and its two helpers all need most of it.
struct Task {
    match_id: MatchId,
    arena: Arena,
    inbox: mpsc::Receiver<ArenaMsg>,
    egress: mpsc::Sender<ArenaOut>,
    stop: watch::Receiver<bool>,
    /// Chunks the world could not take yet, oldest first. Ordering is the whole
    /// point: a black box read back out of `log:match.<id>` must be the sequence
    /// the arena wrote.
    backlog: VecDeque<LedgerChunk>,
    /// `ledger.max_unflushed_chunks + 1`. Past it the arena's own
    /// `Ledger::overflowed` voids the match at the next pulse anyway (nothing is
    /// being acknowledged), so the bound here is only what keeps the task's
    /// memory flat while that happens.
    backlog_bound: usize,
    /// **The bound on one unit's discrete orders waiting for the pulse** (16b cut
    /// M.1, `T-11`): `intents.max_pending_per_unit + 1`, per unit and not per
    /// match, so one pilot's burst cannot crowd out another's orders. Continuous
    /// orders and controls need no bound: they collapse per key (see
    /// `buffer_continuous` and `buffer_control`).
    per_unit_bound: usize,
    /// `tempo.stall_ms`.
    stall: Duration,
    /// **The arena second the last [`ArenaOut::Frames`] carried** (16b cut D.1),
    /// or `None` before the first one. It is the whole of the clock clause: a
    /// pulse with no frame in it sends nothing when the second has not moved.
    last_clock_s: Option<i64>,
    /// **The frames the world could not take** (16b cut M.1, `B-77`), with the
    /// arena's `now_ms` of the newest pulse folded into them, or `None`.
    ///
    /// `Arena::frames` clears the pulse's staged notices and commits each
    /// pilot's view to its last-sent frame when it builds a frame, so a refused
    /// `Frames` is a set of notices that exist nowhere else. `Task::publish`
    /// keeps it here and merges it into the next pulse's (`merge_held`), and
    /// `Task::end` sends what is left before `Ended`. **The rule: a notice is
    /// delivered at most once, and at least once unless the process is
    /// stopping** — merged, never duplicated. The losses are the stop signal,
    /// which owes the world nothing, and, while the world stays behind, the
    /// oldest of a pilot's held notices past [`MAX_HELD_NOTICES`] (counted in
    /// `held_notice_drops`); a frame the world takes when it is built is never
    /// cut.
    held_frames: Option<(i64, Vec<(ObjId, Frame)>)>,
    /// **The staff picture the world could not take** (16b cut I.4), or
    /// `None`. A picture is the whole field at one moment, so a newer one
    /// replaces a held one outright: `Task::publish` sends the pulse's own
    /// picture when it built one and the held one otherwise, and holds again
    /// whatever a full egress refused; `Task::end` sends it with the held
    /// frames.
    held_observer: Option<Observer>,
    /// How many held notices were dropped past [`MAX_HELD_NOTICES`] this match;
    /// the first drop is warned of, once.
    held_notice_drops: u64,
    /// Set the moment the `Ended` send completes; shared with
    /// [`ArenaHandle::has_reported`] (16b cut M.1, `B-49`).
    reported: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// The pre-pulse buffers (16b cut M.1, `B-50`, `T-11`)
// ---------------------------------------------------------------------------

/// The controls waiting for the pulse: one per `(unit, channel)`.
type Controls = BTreeMap<(ObjId, Channel), Control>;

/// The `Released` controls a later control displaced: one per
/// `(unit, channel, pilot)`.
type Releases = BTreeMap<(ObjId, Channel, ObjId), Control>;

/// The continuous orders waiting for the pulse: one per `(unit, kind code)`.
type Continuous = BTreeMap<(ObjId, u8), Intent>;

/// The discrete orders waiting for the pulse, per unit, in arrival order.
type Discrete = BTreeMap<ObjId, Vec<Intent>>;

/// **Buffer one control** (16b cut M.1, `B-50`).
///
/// One control stands per `(unit, channel)`: the new one replaces the standing
/// one when its epoch is **higher or equal** — equal, so a `Released` that
/// arrives after a `Live` at one epoch stands — and is otherwise the one that
/// does not stand. Nothing is thrown away: the control that does not stand, when
/// it is a `Released` naming a pilot, is kept in `releases` under
/// `(unit, channel, pilot)` (a second for the same key replaces the first), so
/// `Released(A)` collapsed away under a later `Live(B)` still takes A off the
/// arena's frame roster. Any other control that does not stand is superseded
/// entirely by the one that does: `apply_control` under highest-epoch-wins
/// raises the table to the same place whichever order the two arrive in.
///
/// Bounded by the seats: at most `units × 3` controls and one release per seat
/// per channel, so neither map needs a bound of its own. Before this cut a
/// control could be dropped as the *oldest* message of a full buffer, which left
/// the world's epoch one above the arena's and every later order from that seat
/// refused `epoch`.
fn buffer_control(controls: &mut Controls, releases: &mut Releases, control: Control) {
    let key = (control.unit, control.channel);
    let not_standing = match controls.get(&key) {
        Some(standing) if control.epoch < standing.epoch => Some(control),
        _ => controls.insert(key, control),
    };
    if let Some(
        displaced @ Control {
            state: ControlState::Released,
            pilot: Some(pilot),
            ..
        },
    ) = not_standing
    {
        releases.insert((displaced.unit, displaced.channel, pilot), displaced);
    }
}

/// The key code a continuous order coalesces under beside its unit.
///
/// **16b cut Z.2** (16c Q44): a `ReleaseOrder` answers `2`, `Order`'s own code.
/// A unit has one standing-instruction slot, and the buffer coalesces per
/// `(unit, code)` and hands continuous orders over in key order and not by
/// `seq`, so a release under a code of its own would always apply after an
/// `ORDER` of the same pulse whichever the pilot typed last. Under one key the
/// higher `seq` wins across both: `HOLD` then `RELEASE` leaves no order, and
/// `RELEASE` then `HOLD` leaves `HOLD`.
const fn continuous_code(kind: &IntentKind) -> u8 {
    match kind {
        IntentKind::Heading { .. } => 0,
        IntentKind::Speed { .. } => 1,
        IntentKind::Order(_) | IntentKind::ReleaseOrder { .. } => 2,
        // Discrete (16b cut I.3: a staff `Place` among them); never buffered
        // under a continuous key.
        IntentKind::Startup
        | IntentKind::Lock { .. }
        | IntentKind::Fire { .. }
        | IntentKind::Place { .. } => 3,
    }
}

/// **Buffer one continuous order** (16b cut M.1, `T-11`): `Heading`, `Speed`
/// and `Order` coalesce per `(unit, kind)`, the higher `seq` winning (a tie goes
/// to the later arrival).
///
/// Nothing a pilot meant is lost: phase 2 applies continuous orders
/// latest-wins, so of two headings in one pulse only the second ever mattered.
fn buffer_continuous(map: &mut Continuous, intent: Intent) {
    let key = (intent.unit, continuous_code(&intent.kind));
    match map.get(&key) {
        Some(standing) if standing.seq > intent.seq => {}
        _ => {
            map.insert(key, intent);
        }
    }
}

/// **Buffer one discrete order** (16b cut M.1, `T-11`): `Startup`, `Lock` and
/// `Fire`, kept per unit in arrival order and bounded **per unit** at
/// `per_unit_bound` (`intents.max_pending_per_unit + 1`).
///
/// When that unit's buffer is full the **newest arrival is refused** — not
/// pushed, one `warn` naming the unit and the kind — and never an older one, so
/// a `LOCK` cannot be dropped while the `FIRE` that needs it survives, and no
/// other unit's orders are touched. Answers whether it was kept.
///
/// **16b cut I.3: a staff `Place` is pushed past the bound and never refused.**
/// The kind is tested before the bound on purpose: the bound is sized for a
/// pilot's burst (one above their rate, so the arena itself can say
/// `queue_full`), and a place is not a pilot's order — refusing it here would
/// drop a staff act because the pilot was busy, and the arena journals every
/// place it receives, so a place dropped here would be the one that leaves no
/// line. Nor does a place count toward the bound a pilot's later order meets,
/// for the same reason the arena's rate does not count it: a pilot's order
/// dropped because staff placed the tank would be a tell. Places are bounded by
/// the world instead — one staff verb is one message — and by the inbox, which
/// holds a place to the reserve rule of an intent.
fn buffer_discrete(
    map: &mut Discrete,
    intent: Intent,
    per_unit_bound: usize,
    match_id: MatchId,
) -> bool {
    let queue = map.entry(intent.unit).or_default();
    if matches!(intent.kind, IntentKind::Place { .. }) {
        queue.push(intent);
        return true;
    }
    let pilots = queue
        .iter()
        .filter(|held| !matches!(held.kind, IntentKind::Place { .. }))
        .count();
    if pilots >= per_unit_bound.max(1) {
        tracing::warn!(
            %match_id,
            unit = %intent.unit,
            kind = intent.kind.as_str(),
            per_unit_bound,
            "an arena's pre-pulse buffer for that unit is full; the newest order was refused"
        );
        return false;
    }
    queue.push(intent);
    true
}

/// **Everything the world offered since the last pulse**, under the four rules
/// above.
#[derive(Default)]
struct PrePulse {
    controls: Controls,
    releases: Releases,
    continuous: Continuous,
    discrete: Discrete,
}

impl PrePulse {
    /// Take the pulse's two slices and leave the buffers empty.
    ///
    /// **Controls**: every kept release first, in key order, then every standing
    /// control in key order — so `Released(A)` runs `drop_pilot(A)` before
    /// `Live(B)` raises the epoch table above it. The rest of the order does not
    /// matter: the table never lowers, and `apply_control` under
    /// highest-epoch-wins leaves it in one place whichever order two controls for
    /// one key arrive in.
    ///
    /// **Intents**: the continuous orders in key order, then each unit's discrete
    /// orders in unit order, each unit's sorted stably by [`Intent::seq`] — the
    /// world's monotonic stamp, so phase 2 counts and queues a burst in the
    /// pilot's order and not the inbox's.
    fn take(&mut self) -> (Vec<Control>, Vec<Intent>) {
        let releases = std::mem::take(&mut self.releases);
        let controls = std::mem::take(&mut self.controls);
        let continuous = std::mem::take(&mut self.continuous);
        let discrete = std::mem::take(&mut self.discrete);
        let controls: Vec<Control> = releases
            .into_values()
            .chain(controls.into_values())
            .collect();
        let mut intents: Vec<Intent> = continuous.into_values().collect();
        for (_, mut orders) in discrete {
            orders.sort_by_key(|order| order.seq);
            intents.extend(orders);
        }
        (controls, intents)
    }
}

// ---------------------------------------------------------------------------
// The held frames (16b cut M.1, `B-77`)
// ---------------------------------------------------------------------------

/// **Merge the frames the world could not take into this pulse's** (16b cut
/// M.1, `B-77`), per pilot, in pilot-id order.
///
/// A pilot in both takes the **new** frame's view with the held frame's
/// `notices` prepended to the new one's; a pilot only in the held message is
/// carried as it is. **Only a pilot who had notices held** has them cut to the
/// last [`MAX_HELD_NOTICES`], the oldest dropped and counted in `drops`: the
/// bound is on what a lagging world lets pile up, never on one pulse, so a
/// frame nothing was held for is never cut — a pilot focus-fired past sixty-four
/// `Struck`s in one pulse on a world that keeps up is told every one. What is
/// held is therefore at most the bound or one pulse's notices, whichever is
/// more. With nothing held the fresh frames keep their own order and are not
/// copied.
///
/// The rule the merge keeps, since **16b cut Y.4**: *a notice, and a catch-up,
/// is delivered at most once, and at least once unless the process is
/// stopping*. A held frame's [`Frame::catchup`] is carried onto the fresh frame
/// when the fresh one has none of its own.
fn merge_held(
    held: Option<(i64, Vec<(ObjId, Frame)>)>,
    fresh: Vec<(ObjId, Frame)>,
    drops: &mut u64,
) -> Vec<(ObjId, Frame)> {
    let Some((_, held)) = held else {
        return fresh;
    };
    // `true` beside a frame that carries notices a lagging world held.
    let mut by_pilot: BTreeMap<ObjId, (Frame, bool)> = held
        .into_iter()
        .map(|(pilot, frame)| (pilot, (frame, true)))
        .collect();
    for (pilot, mut frame) in fresh {
        let carried = match by_pilot.remove(&pilot) {
            Some((mut older, _)) => {
                older.notices.append(&mut frame.notices);
                frame.notices = older.notices;
                // **16b cut Y.4**: a held pulse's catch-up report rides the
                // next frame exactly as its notices do. A reclaim follows a
                // disruption, which is the very moment a world lags, and a
                // fresh view without this would lose the report on every
                // surface. A fresh frame that carries its own keeps it (a
                // second reclaim inside one lag: the newer report is the one
                // measured from the newer drop).
                if frame.catchup.is_none() {
                    frame.catchup = older.catchup.take();
                }
                true
            }
            None => false,
        };
        by_pilot.insert(pilot, (frame, carried));
    }
    by_pilot
        .into_iter()
        .map(|(pilot, (mut frame, carried))| {
            let over = frame.notices.len().saturating_sub(MAX_HELD_NOTICES);
            if carried && over > 0 {
                frame.notices.drain(..over);
                *drops = drops.saturating_add(u64::try_from(over).unwrap_or(u64::MAX));
            }
            (pilot, frame)
        })
        .collect()
}

/// **The staff picture a `Frames` carries** (16b cut I.4): the pulse's own
/// when it built one, else the one a full egress refused before — a picture is
/// the whole field at one moment, so the newer replaces the older outright and
/// nothing is merged.
fn newer_observer(held: Option<Observer>, fresh: Option<Observer>) -> Option<Observer> {
    fresh.or(held)
}

/// **Apply an [`ArenaMsg::Watch`]** (16b cut I.4) to the arena the task owns:
/// `Arena::watched` takes `on`, and a `true` clears `Arena::last_observer_s`, so
/// the next pulse builds a picture for the new watcher whatever second it falls
/// in. Not an input to `step`, and never a pulse of its own.
fn apply_watch(arena: &mut Arena, on: bool) {
    arena.watched = on;
    if on {
        arena.last_observer_s = None;
    }
}

/// **Take a released pilot out of the held frames** (16b cut M.1, `B-77`): a
/// `Control { Released, pilot: Some(p) }` buffered means the world has let `p`
/// go, and nothing held for them is sent afterwards. An emptied hold is `None`.
fn forget_held_pilot(held: &mut Option<(i64, Vec<(ObjId, Frame)>)>, pilot: ObjId) {
    if let Some((_, frames)) = held {
        frames.retain(|(held_pilot, _)| *held_pilot != pilot);
        if frames.is_empty() {
            *held = None;
        }
    }
}

/// **How one waiting send ended** (16b cut M.1, `B-48`): what `Task::end`'s
/// sends answer.
enum Drain {
    /// Everything waiting was delivered.
    Drained,
    /// The stop signal fired (or its sender went) first: the process is
    /// stopping, and the task owes it no result.
    Stopped,
    /// The world's egress is closed; nothing more can be delivered.
    Closed,
}

/// What one pulse asked the loop to do next.
enum Pulse {
    /// Pulse again at this `next_due_ms` — the step's own answer, in the task's
    /// wall milliseconds since its base. The loop turns it into an instant with
    /// [`deadline`], because the base is the loop's (16b cut U.1).
    Again(i64),
    /// The match is over; the task is done.
    Done,
}

/// The task: wait, drain, pulse, publish, repeat.
async fn arena_loop(mut task: Task) {
    // Arena time is measured from the task's own monotonic base, and
    // `next_due_ms` comes back in the same units — so the first pulse is due
    // immediately (it only establishes the wall reference and integrates
    // nothing: doc 16 §3.1 "Clock and lag").
    let base = Instant::now();
    let mut due = base;
    // Every intent and control the world has offered since the last pulse,
    // under the four rules of this module's head (16b cut M.1). Emptied by each
    // pulse, so nothing waiting can outlive the pulse that takes it.
    let mut waiting = PrePulse::default();
    loop {
        tokio::select! {
            // The stop signal wins every race: a process that is stopping does
            // not owe the world one more pulse.
            biased;
            changed = task.stop.changed() => {
                if changed.is_err() || *task.stop.borrow_and_update() {
                    tracing::debug!(match_id = %task.match_id, "arena task stopped by signal");
                    return;
                }
            }
            msg = task.inbox.recv() => {
                match msg {
                    // Buffered, never stepped: the pulse is the only thing that
                    // applies an order (doc 16 §3.1 phases 1 and 2).
                    Some(ArenaMsg::Control(control)) => {
                        // 16b cut M.1 (`B-77`): a pilot the world has let go is
                        // sent nothing that was held for them.
                        if let Control {
                            state: ControlState::Released,
                            pilot: Some(pilot),
                            ..
                        } = control
                        {
                            forget_held_pilot(&mut task.held_frames, pilot);
                        }
                        buffer_control(&mut waiting.controls, &mut waiting.releases, control);
                    }
                    Some(ArenaMsg::Intent(intent)) => {
                        if intent.kind.is_discrete() {
                            let _ = buffer_discrete(
                                &mut waiting.discrete,
                                intent,
                                task.per_unit_bound,
                                task.match_id,
                            );
                        } else {
                            buffer_continuous(&mut waiting.continuous, intent);
                        }
                    }
                    // A `Stop` is recorded now and honoured by the next pulse's
                    // phase 1, which is the library's own rule — it never lands
                    // in the middle of a substep. **16b cut I.3**: and that pulse
                    // is pulled in to a millisecond away, so an abort under a
                    // ten-second floor is not ten seconds late (this module's
                    // head).
                    Some(ArenaMsg::Stop(reason)) => {
                        task.arena.stop(reason);
                        due = pulled_in(due);
                    }
                    // An acknowledgement is not an input to `step`: it only
                    // lowers the egress's depth, so it applies on arrival.
                    Some(ArenaMsg::LedgerAck { upto_chunk_seq }) => {
                        task.arena.ledger.mark_flushed(upto_chunk_seq);
                    }
                    // **16b cut I.3** (16c N10): nor is a staff floor. It is
                    // applied on arrival, and the deadline is pulled in so the
                    // set or the lift takes effect at the next millisecond: the
                    // loop otherwise recomputes `due` only after a pulse, and the
                    // old wake may be a Quiet period or a floor away. The pulled
                    // pulse is quantised to the lattice like every floored one.
                    Some(ArenaMsg::TempoFloor { period_ms }) => {
                        task.arena.clock.set_floor(period_ms);
                        due = pulled_in(due);
                    }
                    // **16b cut I.4** (16c N10): nor is a watch. The next
                    // pulse, whenever it falls, builds the picture; the
                    // deadline is left alone, because a picture is a view and
                    // the arena's cadence is not the watcher's to move.
                    Some(ArenaMsg::Watch { on }) => apply_watch(&mut task.arena, on),
                    // Every sender is gone: the world has forgotten this match
                    // and nothing can ever drive it again. End silently, for the
                    // same reason the stop signal does.
                    None => {
                        tracing::debug!(
                            match_id = %task.match_id,
                            "arena task ended: the world dropped its inbox"
                        );
                        return;
                    }
                }
            }
            () = tokio::time::sleep_until(due) => {
                let wall = elapsed_ms(base);
                let (controls, intents) = waiting.take();
                match task.pulse(wall, &controls, &intents).await {
                    // 16b cut U.1: from the base, never from now.
                    Pulse::Again(next_due_ms) => due = deadline(base, next_due_ms),
                    Pulse::Done => return,
                }
            }
        }
    }
}

impl Task {
    /// **One pulse**: the unwind boundary, the watchdog, and the publication of
    /// whatever the pulse produced.
    async fn pulse(&mut self, wall_now_ms: i64, controls: &[Control], intents: &[Intent]) -> Pulse {
        // First, whatever the world could not take last time — in order, ahead
        // of anything this pulse closes.
        self.drain_backlog();

        let started = Instant::now();
        let stepped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.arena.step(wall_now_ms, controls, intents)
        }));
        let took = started.elapsed();

        let Ok(out) = stepped else {
            // Doc 16 §3.1 "Failure". The `StepOutput` does not exist, and the
            // arena's own chunk egress is deliberately left untouched: nothing
            // the faulted pulse produced leaves. The digest describes the black
            // box as it stood, which commits nothing — a voided match returns no
            // cell to the world at all (T.3's check-in restores the snapshots).
            tracing::error!(
                match_id = %self.match_id,
                arena_ms = self.arena.now_ms,
                "ARENA STEP PANIC caught at the task's unwind boundary; the match is void"
            );
            self.end(MatchEnd {
                reason: EndReason::Void,
                t_ms: self.arena.now_ms,
            })
            .await;
            return Pulse::Done;
        };

        let ended = out.ended;
        // Read out of the `StepOutput` before `publish` consumes it: it is the
        // pulse's own answer to "when again", in the same wall milliseconds
        // `wall_now_ms` was given in (doc 16 §3.1 phase 5).
        let next_due_ms = out.next_due_ms;
        self.publish(out).await;

        // **16b cut AA.3 (16c `B-79`)**: the watchdog is read BEFORE the end is
        // honoured. A pulse that overran `tempo.stall_ms` and also ended the match
        // used to be accepted with its own reason and its arena-derived cells
        // committed, where doc 16 §3.1 says a stalled pulse is void and every unit
        // goes back to its check-out snapshot. The void is forced HERE and not asked
        // of the arena: `Arena::stop` is first-wins and `Arena::step` echoes an
        // already-recorded end before it reads the stop, so the library's own
        // stopping pulse would hand the original reason straight back. Nothing is
        // stepped again; `Task::end` sends no units for a `Void`, and the world
        // restores the snapshots. The black box therefore keeps the arena's own
        // end wherever it journalled one (`match_ended`, or `aborted` for an
        // abort; a ledger-full timeout journals none) and no `void` event, while
        // the result says `void` — recorded in 16b's AA row; the journalled
        // variant is a library change and larger.
        let stalled = took > self.stall;
        if let Some(end) = ended {
            // An end that is already a void keeps its word: both reasons are void
            // on both sides of the seam (`Task::end` and `match_::finish` test the
            // same pair), and the overflow's `void` event is already in the black
            // box — there is nothing truer to say.
            let already_void = matches!(end.reason, EndReason::Void | EndReason::LedgerOverflow);
            if stalled && !already_void {
                tracing::error!(
                    match_id = %self.match_id,
                    took_ms = took.as_millis(),
                    stall_ms = self.stall.as_millis(),
                    reason = ?end.reason,
                    arena_ms = end.t_ms,
                    "arena pulse exceeded tempo.stall_ms on the pulse that ended the match; the match is void and its units are restored"
                );
                self.end(MatchEnd {
                    reason: EndReason::Void,
                    t_ms: self.arena.now_ms,
                })
                .await;
            } else {
                self.end(end).await;
            }
            return Pulse::Done;
        }

        // The watchdog reads the clock *after* `step` returns, because the arena
        // has no preemption: a pulse cannot be cut short, only denied a
        // successor. The stopping pulse below is the library's own `Stop` path,
        // so the `void` event and the ledger's tail are recorded the same way an
        // aborted match records them.
        if stalled {
            tracing::error!(
                match_id = %self.match_id,
                took_ms = took.as_millis(),
                stall_ms = self.stall.as_millis(),
                "arena pulse exceeded tempo.stall_ms; the match is void"
            );
            self.arena.stop(StopReason::Void);
            let stopping = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.arena.step(wall_now_ms, &[], &[])
            }));
            let end = match stopping {
                Ok(out) => {
                    let end = out.ended;
                    self.publish(out).await;
                    end
                }
                Err(_) => None,
            };
            self.end(end.unwrap_or(MatchEnd {
                reason: EndReason::Void,
                t_ms: self.arena.now_ms,
            }))
            .await;
            return Pulse::Done;
        }

        Pulse::Again(next_due_ms)
    }

    /// Publish one pulse's output: its chunks in order, then its frames, then
    /// the lag notice.
    async fn publish(&mut self, out: StepOutput) {
        let StepOutput {
            ledger_chunks,
            frames,
            lagged,
            deliberation,
            observer,
            ..
        } = out;
        for chunk in ledger_chunks {
            self.queue_chunk(chunk);
        }
        self.drain_backlog();
        // **16b cut M.1** (`B-77`): whatever the world could not take last time,
        // merged in — the newest view, every notice once, in order.
        let frames = self.merge_held(frames);
        // **16b cut D.1**: the message carries the arena's clock, so it is sent
        // when there is a frame **or** when the arena second has moved since the
        // last one — at most one empty message an arena second, whatever the
        // pulse rate. See [`ArenaOut::Frames`].
        let now_ms = self.arena.now_ms;
        let second = now_ms.div_euclid(1_000);
        // **16b cut I.4**: the pulse's own staff picture, else the one a full
        // egress refused last time — the newer always wins, and a held picture
        // older than the pulse's is simply dropped.
        let observer = newer_observer(self.held_observer.take(), observer);
        if !frames.is_empty() || self.last_clock_s != Some(second) || observer.is_some() {
            self.last_clock_s = Some(second);
            // Never awaited: the world task must not be waited on by a pulse. A
            // full egress keeps the message here instead, and the next pulse
            // merges into it (see `Task::held_frames`); `end` is the one place
            // that waits. An empty message is only the clock, which the next
            // pulse's supersedes, so it is not worth holding; a closed egress is
            // a world that has gone. **16b cut I.4**: a refused picture is held
            // on its own (`Task::held_observer`), frames or none.
            if let Err(TrySendError::Full(ArenaOut::Frames {
                frames,
                now_ms,
                observer,
                ..
            })) = self.egress.try_send(ArenaOut::Frames {
                match_id: self.match_id,
                frames,
                now_ms,
                observer,
            }) {
                if !frames.is_empty() {
                    self.held_frames = Some((now_ms, frames));
                }
                self.held_observer = observer;
            }
        }
        if lagged {
            let _ = self.egress.try_send(ArenaOut::Lagged {
                match_id: self.match_id,
            });
        }
        // 16b cut N.2: droppable for the same reason frames are — a signal the
        // world could not take is re-sent by the next pulse whose digest still
        // differs, and the arena never learns either way.
        if !deliberation.is_empty() {
            let _ = self.egress.try_send(ArenaOut::Deliberate {
                match_id: self.match_id,
                signals: deliberation,
            });
        }
    }

    /// **Merge the held frames into this pulse's** (16b cut M.1, `B-77`), and
    /// warn once a match the first time a pilot's held notices overflow.
    fn merge_held(&mut self, fresh: Vec<(ObjId, Frame)>) -> Vec<(ObjId, Frame)> {
        let before = self.held_notice_drops;
        let merged = merge_held(self.held_frames.take(), fresh, &mut self.held_notice_drops);
        if before == 0 && self.held_notice_drops > 0 {
            tracing::warn!(
                match_id = %self.match_id,
                max = MAX_HELD_NOTICES,
                "the world has not taken this match's frames for long enough that a pilot's \
                 held notices passed the bound; the oldest were dropped"
            );
        }
        merged
    }

    /// **Hand the world the end of the match**, waiting for room if it needs
    /// some — but never past the stop signal, which is the process shutting
    /// down.
    ///
    /// **16b cut M.1** (`B-48`, `B-77`): three waits, in order, each racing the
    /// stop signal:
    ///
    /// 1. **every queued ledger chunk** (`drain_backlog_waiting`) — before this
    ///    cut the backlog was offered once with `try_send`, so under backpressure
    ///    the result recorded a digest `log:match.<id>` could not reproduce. On
    ///    the panic arm and the stalled-terminal arm this is the black-box
    ///    evidence: the chunks that closed before the fault are exactly what an
    ///    operator reads to learn what happened, so they wait too;
    /// 2. **the held frames**, if any (`Task::held_frames`) — the final pulse's
    ///    notices, the killing hit, `Kill` and `OwnDestroyed` among them, reach a
    ///    lagging world;
    /// 3. **the `Ended`**, whose digest is `Ledger::shipped_digest`.
    ///
    /// A **stopped** wait sends no `Ended` at all: the stop lever means a process
    /// that is stopping owes no result. It must return rather than fall through
    /// to the next send, because a `watch` whose change a wait already took with
    /// `borrow_and_update` never fires again, and a later send on a full egress
    /// would then never finish — and the shutdown tail's join with it. A closed
    /// egress returns too: nothing more can be delivered.
    async fn end(&mut self, end: MatchEnd) {
        match self.drain_backlog_waiting().await {
            Drain::Drained => {}
            Drain::Stopped => {
                tracing::warn!(
                    match_id = %self.match_id,
                    "arena task stopped before it could report its result"
                );
                return;
            }
            Drain::Closed => {
                tracing::warn!(
                    match_id = %self.match_id,
                    "arena result dropped: the world's egress is closed"
                );
                return;
            }
        }
        // **16b cut I.4**: a held staff picture rides the held frames, or goes
        // on its own when no frame was held.
        let held_observer = self.held_observer.take();
        let held_frames = self.held_frames.take();
        if held_frames.is_some() || held_observer.is_some() {
            let (now_ms, frames) = held_frames.unwrap_or((self.arena.now_ms, Vec::new()));
            let held = ArenaOut::Frames {
                match_id: self.match_id,
                frames,
                now_ms,
                observer: held_observer,
            };
            match self.send_waiting(held).await {
                Drain::Drained => {}
                Drain::Stopped => {
                    tracing::warn!(
                        match_id = %self.match_id,
                        "arena task stopped before it could report its result"
                    );
                    return;
                }
                Drain::Closed => {
                    tracing::warn!(
                        match_id = %self.match_id,
                        "arena result dropped: the world's egress is closed"
                    );
                    return;
                }
            }
        }
        // Doc 16 §3.1 "Failure": a void commits nothing the arena derived, so a void
        // carries no units at all and the world restores every check-out snapshot.
        // The judgement is made **here**, where the reason is, rather than trusted to
        // the far end of the seam.
        let void = matches!(end.reason, EndReason::Void | EndReason::LedgerOverflow);
        let message = ArenaOut::Ended {
            match_id: self.match_id,
            end,
            // 16b cut M.1 (`B-48`): the digest of what left, which is what the
            // log can reproduce — see [`ArenaOut::Ended`].
            digest: self.arena.ledger.shipped_digest(),
            units: if void {
                Vec::new()
            } else {
                self.arena.returned_units()
            },
        };
        // The one message the task does wait to deliver: a result the world
        // never hears is a match whose units are never checked back in.
        match self.send_waiting(message).await {
            Drain::Drained => {
                // 16b cut M.1 (`B-49`): from here the result is the world's, and
                // a stop that finds this task's inbox closed is `Busy`, not
                // `Gone`.
                self.reported.store(true, Ordering::Release);
            }
            Drain::Stopped => tracing::warn!(
                match_id = %self.match_id,
                "arena task stopped before it could report its result"
            ),
            Drain::Closed => tracing::warn!(
                match_id = %self.match_id,
                "arena result dropped: the world's egress is closed"
            ),
        }
    }

    /// **Send one message, waiting for room, racing the stop signal** (16b cut
    /// M.1). The loop is for a `watch` that changed without going `true` — that
    /// is not a shutdown and must not lose the message.
    async fn send_waiting(&mut self, message: ArenaOut) -> Drain {
        loop {
            tokio::select! {
                biased;
                changed = self.stop.changed() => {
                    if changed.is_err() || *self.stop.borrow_and_update() {
                        return Drain::Stopped;
                    }
                }
                sent = self.egress.send(message.clone()) => {
                    return if sent.is_err() { Drain::Closed } else { Drain::Drained };
                }
            }
        }
    }

    /// **Deliver every queued ledger chunk, oldest first, waiting for room**
    /// (16b cut M.1, `B-48`): `Drained` when the backlog is empty, `Stopped` at
    /// the stop signal, `Closed` when the egress is. A chunk leaves the backlog
    /// only once the egress has taken it, so the order is never broken. The
    /// `try_send` form, `drain_backlog`, stays for `pulse` and `publish`, which
    /// must never wait.
    async fn drain_backlog_waiting(&mut self) -> Drain {
        while let Some(chunk) = self.backlog.front().cloned() {
            let message = ArenaOut::Ledger {
                match_id: self.match_id,
                chunk,
            };
            match self.send_waiting(message).await {
                Drain::Drained => {
                    self.backlog.pop_front();
                }
                other => return other,
            }
        }
        Drain::Drained
    }

    /// Put one closed chunk at the back of the retry queue.
    fn queue_chunk(&mut self, chunk: LedgerChunk) {
        if self.backlog.len() >= self.backlog_bound {
            // The world has stopped draining. The arena's own
            // `Ledger::overflowed` voids the match at the next pulse (nothing
            // is being acknowledged either), so this only keeps the task's
            // memory flat until it does; the oldest goes, because the newest
            // chunks are the ones nearest the failure worth reading.
            let dropped = self.backlog.pop_front().map(|chunk| chunk.seq);
            tracing::error!(
                match_id = %self.match_id,
                dropped = ?dropped,
                "the world is not draining the match ledger; a chunk was dropped"
            );
        }
        self.backlog.push_back(chunk);
    }

    /// Offer as much of the retry queue as the egress will take, oldest first,
    /// stopping at the first refusal so the order is never broken.
    fn drain_backlog(&mut self) {
        while let Some(chunk) = self.backlog.front() {
            let message = ArenaOut::Ledger {
                match_id: self.match_id,
                chunk: chunk.clone(),
            };
            match self.egress.try_send(message) {
                Ok(()) => {
                    self.backlog.pop_front();
                }
                Err(_) => return,
            }
        }
    }
}

/// When to pulse again, from the pulse's own `next_due_ms`: the task's base
/// plus that many milliseconds, or one millisecond from now if that has already
/// passed.
///
/// **16b cut U.1** (16c `T-22`): the deadline is **absolute**. `next_due_ms` is
/// in the task's own wall milliseconds since `base` (the reading `step` was given
/// is `elapsed_ms(base)`), and `Arena::next_due_ms` answers a lattice point of
/// arena time in them, so the deadline is `base + next_due_ms`. It used to be
/// the interval `next_due_ms − wall_now_ms` measured from *now*, the instant the
/// pulse finished — which added the pulse's own cost, and the timer's overshoot
/// on the wake before it, to arena time on every wake, so the task drifted off
/// the lattice by a little more each pulse and never came back.
///
/// Arena time is still wall time (doc 16 §3.1 "Clock and lag"): a production
/// wake is the lattice point plus the OS timer's overshoot, which §3.1 calls wall
/// jitter, and the pulse it starts ends that much off the lattice. The overshoot
/// no longer **accumulates**, because the next deadline is again a point from the
/// base and `Arena::next_due_ms` pulls a late arena back onto the lattice. (The
/// task does not hand `step` the scheduled wall instead of the reading; that
/// would make arena time exactly the lattice with lateness carried as lag, and
/// is an owner question, not this cut's.)
///
/// **16b cut M.2** (16c `B-51`): the floor is one millisecond from now, never
/// zero. A zero sleep re-pulses inside the same wall millisecond, whose `dt` is
/// `0` and integrates nothing, so an event due "now" spun the timer instead of
/// letting arena time reach it; and an arena that is behind still gets the
/// shortest sleep there is, with the next pulse's own clamp (doc 16 §3.1
/// "Clock and lag") absorbing the lag.
fn deadline(base: Instant, next_due_ms: i64) -> Instant {
    let at = base + Duration::from_millis(u64::try_from(next_due_ms).unwrap_or(0));
    at.max(Instant::now() + Duration::from_millis(1))
}

/// **A deadline pulled in to a millisecond from now** (16b cut I.3), or the one
/// already standing when it is sooner — what a `Stop` and a `TempoFloor` do to
/// the loop's next wake. A millisecond and not nought, for [`deadline`]'s reason.
fn pulled_in(due: Instant) -> Instant {
    due.min(Instant::now() + Duration::from_millis(1))
}

/// Milliseconds since the task's monotonic base, saturating.
fn elapsed_ms(base: Instant) -> i64 {
    i64::try_from(base.elapsed().as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    //! **16b cut M.1**: the buffer and hold rules, as the pure functions they
    //! are — no runtime, no task.

    use super::*;
    use crate::arena::perception::FrameNotice;
    use crate::arena::state::Team;
    use crate::arena::test_support::{arena_at_epoch, pose, tank};
    use smol_str::SmolStr;

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    const A: ObjId = ObjId(101);
    const B: ObjId = ObjId(102);

    fn control(epoch: u64, state: ControlState, pilot: Option<ObjId>) -> Control {
        Control {
            unit: RED,
            channel: Channel::Helm,
            epoch,
            state,
            pilot,
        }
    }

    fn order(unit: ObjId, kind: IntentKind, seq: u64) -> Intent {
        Intent {
            unit,
            channel: kind.channel_required(),
            epoch: 1,
            kind,
            seq,
        }
    }

    /// Seventeen rising epochs and a `Released` in one window: one control
    /// stands, at the highest epoch, and nothing is dropped.
    #[test]
    fn m1_controls_collapse_to_the_highest_epoch_and_are_never_dropped() {
        let mut controls = Controls::new();
        let mut releases = Releases::new();
        for epoch in 1..=17 {
            buffer_control(
                &mut controls,
                &mut releases,
                control(epoch, ControlState::Live, None),
            );
        }
        buffer_control(
            &mut controls,
            &mut releases,
            control(18, ControlState::Released, None),
        );
        assert_eq!(controls.len(), 1);
        assert_eq!(
            controls.get(&(RED, Channel::Helm)),
            Some(&control(18, ControlState::Released, None))
        );
        // A release naming nobody is superseded, not kept: it changes no roster.
        assert!(releases.is_empty());

        // An equal epoch: a `Released` after a `Live` stands.
        let mut controls = Controls::new();
        buffer_control(
            &mut controls,
            &mut releases,
            control(3, ControlState::Live, Some(A)),
        );
        buffer_control(
            &mut controls,
            &mut releases,
            control(3, ControlState::Released, Some(A)),
        );
        assert_eq!(
            controls.get(&(RED, Channel::Helm)).map(|c| c.state),
            Some(ControlState::Released)
        );
    }

    /// `Released(A)` displaced by `Live(B)` is kept aside and drained FIRST; a
    /// late, lower-epoch `Released(A)` is kept too.
    #[test]
    fn m1_a_displaced_release_is_kept_and_drained_before_the_controls() {
        let mut waiting = PrePulse::default();
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            control(4, ControlState::Released, Some(A)),
        );
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            control(5, ControlState::Live, Some(B)),
        );
        assert_eq!(
            waiting.releases.get(&(RED, Channel::Helm, A)),
            Some(&control(4, ControlState::Released, Some(A)))
        );
        // The refused newcomer: a release older than what stands.
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            Control {
                channel: Channel::Fire,
                ..control(9, ControlState::Live, Some(B))
            },
        );
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            Control {
                channel: Channel::Fire,
                ..control(2, ControlState::Released, Some(A))
            },
        );
        assert_eq!(waiting.releases.len(), 2);
        let (controls, intents) = waiting.take();
        assert!(intents.is_empty());
        assert_eq!(
            controls
                .iter()
                .map(|c| (c.channel, c.epoch, c.state))
                .collect::<Vec<_>>(),
            vec![
                (Channel::Helm, 4, ControlState::Released),
                (Channel::Fire, 2, ControlState::Released),
                (Channel::Helm, 5, ControlState::Live),
                (Channel::Fire, 9, ControlState::Live),
            ],
            "every release, in key order, then every standing control"
        );
        // And the buffers are empty after the take.
        assert!(waiting.controls.is_empty() && waiting.releases.is_empty());
    }

    /// **16b cut Y.3**: the drop's `Released(2, A)` reaching the buffer after
    /// the reclaim's `Live(3, A)` is kept, and the pulse takes it first — so the
    /// arena drops A and then re-seats A, and the tank ends A's whichever order
    /// the two left the world in.
    #[test]
    fn y3_a_late_release_behind_a_reclaim_is_kept_and_taken_first() {
        let mut waiting = PrePulse::default();
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            control(3, ControlState::Live, Some(A)),
        );
        buffer_control(
            &mut waiting.controls,
            &mut waiting.releases,
            control(2, ControlState::Released, Some(A)),
        );
        assert_eq!(
            waiting.releases.get(&(RED, Channel::Helm, A)),
            Some(&control(2, ControlState::Released, Some(A)))
        );
        let (controls, intents) = waiting.take();
        assert!(intents.is_empty());
        assert_eq!(
            controls
                .iter()
                .map(|c| (c.epoch, c.state, c.pilot))
                .collect::<Vec<_>>(),
            vec![
                (2, ControlState::Released, Some(A)),
                (3, ControlState::Live, Some(A)),
            ],
            "the release first, then the reclaim"
        );
    }

    /// Two headings and two speeds for one unit coalesce to the last of each by
    /// `seq`; an older one arriving late does not win.
    #[test]
    fn m1_continuous_orders_coalesce_per_unit_and_kind_to_the_highest_seq() {
        let mut map = Continuous::new();
        buffer_continuous(&mut map, order(RED, IntentKind::Heading { mdeg: 10 }, 1));
        buffer_continuous(&mut map, order(RED, IntentKind::Speed { cms: 100 }, 2));
        buffer_continuous(&mut map, order(RED, IntentKind::Heading { mdeg: 20 }, 3));
        buffer_continuous(&mut map, order(RED, IntentKind::Speed { cms: 200 }, 5));
        buffer_continuous(&mut map, order(RED, IntentKind::Speed { cms: 150 }, 4));
        buffer_continuous(&mut map, order(BLUE, IntentKind::Heading { mdeg: 30 }, 6));
        let kept: Vec<(ObjId, IntentKind)> = map
            .into_values()
            .map(|intent| (intent.unit, intent.kind))
            .collect();
        assert_eq!(
            kept,
            vec![
                (RED, IntentKind::Heading { mdeg: 20 }),
                (RED, IntentKind::Speed { cms: 200 }),
                (BLUE, IntentKind::Heading { mdeg: 30 }),
            ]
        );
    }

    /// **16b cut Z.2 (16c Q44)**: an `ORDER` and a `RELEASE` for one unit in one
    /// pulse share `Order`'s buffer key, so through the task's own buffer the
    /// pilot's last word is what applies: `HOLD` then `RELEASE` leaves no order
    /// after the pulse (the release drops the order the machine already stood
    /// under), and `RELEASE` then `HOLD` leaves `HOLD`.
    #[test]
    fn z2_an_order_and_a_release_in_one_pulse_apply_in_the_pilots_order() {
        use crate::deliberation::order::{Constraints, Objective, Order, StandingOrder};
        use crate::deliberation::squad::{SquadId, SquadRoster};
        let squad = SquadId {
            team: Team::Red,
            index: 1,
        };
        let told = |objective| Order {
            squad,
            objective,
            target: None,
            horizon_ms: 600_000,
            constraints: Constraints::default(),
            ttl_ms: 600_000,
            issued_ms: 0,
            issuer: Some(A),
            spot: None,
        };
        let hold = || IntentKind::Order(Box::new(told(Objective::Hold)));
        let release = || IntentKind::ReleaseOrder {
            squad,
            issuer: Some(A),
        };
        for (first, second, left) in [(hold(), release(), None), (release(), hold(), Some("hold"))]
        {
            let mut arena = arena_at_epoch(vec![tank(RED, Team::Red, pose(1_500, 1_500))], 1);
            if let Some(state) = arena.unit_mut(RED) {
                state.controller = crate::arena::state::Controller::Reflex;
            }
            arena
                .reflex
                .adopt(RED, crate::arena::reflex::ReflexState::Patrol);
            // The machine already stands under the squad's `Engage`.
            arena.reflex.set_order(
                RED,
                StandingOrder::accepted(told(Objective::Engage { target_hint: None }), 0),
            );
            arena.squads = vec![SquadRoster {
                id: squad,
                commander: RED,
                units: vec![RED],
            }];
            let mut waiting = PrePulse::default();
            buffer_continuous(&mut waiting.continuous, order(RED, first, 1));
            buffer_continuous(&mut waiting.continuous, order(RED, second, 2));
            let (controls, intents) = waiting.take();
            assert_eq!(intents.len(), 1, "one standing-instruction slot per unit");
            let _ = arena.step(0, &controls, &intents);
            assert_eq!(
                arena
                    .reflex
                    .order(RED)
                    .map(|standing| standing.order.objective.as_str()),
                left
            );
        }
    }

    /// The discrete bound is per unit and refuses the NEWEST arrival; another
    /// unit's orders are untouched; the take sorts each unit's by `seq`.
    #[test]
    fn m1_discrete_orders_are_bounded_per_unit_and_the_newest_is_refused() {
        let mut waiting = PrePulse::default();
        let lock = |seq| {
            order(
                RED,
                IntentKind::Lock {
                    contact: crate::arena::state::ContactId(7),
                },
                seq,
            )
        };
        let fire = |seq| {
            order(
                RED,
                IntentKind::Fire {
                    weapon: SmolStr::new("cannon"),
                },
                seq,
            )
        };
        // Arrival out of seq order, to see the sort.
        assert!(buffer_discrete(
            &mut waiting.discrete,
            fire(2),
            3,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            lock(1),
            3,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(BLUE, IntentKind::Startup, 3),
            3,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            lock(4),
            3,
            MatchId(1)
        ));
        // RED is full at three: the newest is refused, never an older one.
        assert!(!buffer_discrete(
            &mut waiting.discrete,
            fire(5),
            3,
            MatchId(1)
        ));
        // BLUE still has room.
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(BLUE, IntentKind::Startup, 6),
            3,
            MatchId(1)
        ));
        buffer_continuous(
            &mut waiting.continuous,
            order(BLUE, IntentKind::Speed { cms: 1 }, 7),
        );
        let (controls, intents) = waiting.take();
        assert!(controls.is_empty());
        assert_eq!(
            intents
                .iter()
                .map(|intent| (intent.unit, intent.seq))
                .collect::<Vec<_>>(),
            vec![
                (BLUE, 7),
                (RED, 1),
                (RED, 2),
                (RED, 4),
                (BLUE, 3),
                (BLUE, 6)
            ],
            "the continuous orders first, then each unit's discrete ones in unit \
             order, each sorted by seq"
        );
    }

    /// One frame per pilot of a two-tank arena, straight out of its opening pulse.
    fn frames() -> Vec<(ObjId, Frame)> {
        let mut arena = arena_at_epoch(
            vec![
                tank(RED, Team::Red, pose(1_500, 1_500)),
                tank(BLUE, Team::Blue, pose(6_500, 6_500)),
            ],
            1,
        );
        let out = arena.step(0, &[], &[]);
        assert_eq!(
            out.frames.len(),
            2,
            "both pilots are sent their opening frame"
        );
        out.frames
    }

    fn miss(n: usize) -> FrameNotice {
        FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new(format!("n{n}")),
        }
    }

    fn with(frame: &Frame, notices: Vec<FrameNotice>, x_cm: i64) -> Frame {
        let mut frame = frame.clone();
        frame.notices = notices;
        frame.pose.x_cm = x_cm;
        frame
    }

    /// The held notices go first, the new view wins, a pilot only in the held
    /// message is carried, and the output is in pilot order.
    #[test]
    fn m1_merge_held_keeps_every_notice_in_order_and_takes_the_newest_view() {
        let base = frames();
        let (p0, f0) = base[0].clone();
        let (p1, f1) = base[1].clone();
        let held = Some((
            100,
            vec![
                (p0, with(&f0, vec![miss(1), miss(2)], 10)),
                (p1, with(&f1, vec![miss(3)], 20)),
            ],
        ));
        let fresh = vec![(p0, with(&f0, vec![miss(4)], 11))];
        let mut drops = 0;
        let merged = merge_held(held, fresh, &mut drops);
        assert_eq!(drops, 0);
        assert_eq!(
            merged.iter().map(|(pilot, _)| *pilot).collect::<Vec<_>>(),
            vec![p0, p1]
        );
        assert_eq!(merged[0].1.notices, vec![miss(1), miss(2), miss(4)]);
        assert_eq!(merged[0].1.pose.x_cm, 11, "the view is the newer frame's");
        assert_eq!(merged[1].1.notices, vec![miss(3)]);
        assert_eq!(merged[1].1.pose.x_cm, 20, "carried as it was");

        // Nothing held: the fresh frames pass through in their own order and
        // are never cut — seventy notices a world that keeps up can take are
        // seventy notices told.
        let fresh = vec![
            (p0, with(&f0, vec![miss(5)], 12)),
            (p1, with(&f1, (0..70).map(miss).collect(), 13)),
        ];
        let merged = merge_held(None, fresh, &mut drops);
        assert_eq!(merged[0].1.notices, vec![miss(5)]);
        assert_eq!(merged[1].1.notices, (0..70).map(miss).collect::<Vec<_>>());
        assert_eq!(drops, 0);

        // Something held for one pilot only: the other pilot's fresh frame is
        // not cut either, however long it is.
        let held = Some((200, vec![(p0, with(&f0, vec![miss(1)], 14))]));
        let fresh = vec![
            (p0, with(&f0, vec![miss(2)], 15)),
            (p1, with(&f1, (0..70).map(miss).collect(), 16)),
        ];
        let merged = merge_held(held, fresh, &mut drops);
        assert_eq!(merged[0].1.notices, vec![miss(1), miss(2)]);
        assert_eq!(merged[1].1.notices.len(), 70);
        assert_eq!(drops, 0);
    }

    /// A catch-up report measured `away_ms` long, and nothing else.
    fn report(away_ms: i64) -> crate::arena::catchup::Catchup {
        crate::arena::catchup::Catchup {
            away_ms,
            cell_then: Some((1, 1)),
            cell_now: Some((2, 2)),
            power_then: crate::arena::state::Power::Online,
            power_now: crate::arena::state::Power::Online,
            wreck: false,
            contacts_new: Vec::new(),
            contacts_lost: Vec::new(),
            contacts_more: 0,
            sections: Vec::new(),
            sections_more: 0,
            rounds_spent: Vec::new(),
            rounds_more: 0,
            machine: None,
        }
    }

    /// **16b cut Y.4**: a held frame's catch-up rides the fresh frame that
    /// has none, once, beside the held notices; a fresh frame with its own
    /// keeps its own; and a pilot with nothing held is untouched.
    #[test]
    fn y4_merge_held_carries_a_held_catch_up_and_keeps_a_fresh_one() {
        let base = frames();
        let (p0, f0) = base[0].clone();
        let (p1, f1) = base[1].clone();
        let mut held_frame = with(&f0, vec![miss(1)], 10);
        held_frame.catchup = Some(report(20_000));
        let held = Some((100, vec![(p0, held_frame)]));
        let fresh = vec![
            (p0, with(&f0, vec![miss(2)], 11)),
            (p1, with(&f1, Vec::new(), 12)),
        ];
        let mut drops = 0;
        let merged = merge_held(held, fresh, &mut drops);
        assert_eq!(merged.len(), 2, "one frame a pilot");
        assert_eq!(merged[0].1.catchup, Some(report(20_000)), "carried");
        assert_eq!(merged[0].1.notices, vec![miss(1), miss(2)]);
        assert_eq!(merged[0].1.pose.x_cm, 11, "the view is the newer frame's");
        assert_eq!(merged[1].1.catchup, None);

        // A fresh frame with its own keeps its own.
        let mut held_frame = with(&f0, Vec::new(), 10);
        held_frame.catchup = Some(report(20_000));
        let mut fresh_frame = with(&f0, Vec::new(), 11);
        fresh_frame.catchup = Some(report(3_000));
        let merged = merge_held(
            Some((100, vec![(p0, held_frame)])),
            vec![(p0, fresh_frame)],
            &mut drops,
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1.catchup, Some(report(3_000)));
        assert_eq!(drops, 0);
    }

    /// Past sixty-four a pilot, the oldest go and are counted.
    #[test]
    fn m1_merge_held_keeps_the_last_sixty_four_notices_a_pilot() {
        let base = frames();
        let (p0, f0) = base[0].clone();
        let held = Some((100, vec![(p0, with(&f0, (0..60).map(miss).collect(), 10))]));
        let fresh = vec![(p0, with(&f0, (60..70).map(miss).collect(), 11))];
        let mut drops = 0;
        let merged = merge_held(held, fresh, &mut drops);
        assert_eq!(merged[0].1.notices.len(), MAX_HELD_NOTICES);
        assert_eq!(drops, 6);
        assert_eq!(
            merged[0].1.notices.first(),
            Some(&miss(6)),
            "the oldest went"
        );
        assert_eq!(merged[0].1.notices.last(), Some(&miss(69)));
    }

    /// A released pilot leaves the hold; an emptied hold is `None`.
    #[test]
    fn m1_a_released_pilot_is_taken_out_of_the_held_frames() {
        let base = frames();
        let (p0, f0) = base[0].clone();
        let (p1, f1) = base[1].clone();
        let mut held = Some((100, vec![(p0, f0), (p1, f1)]));
        forget_held_pilot(&mut held, p1);
        assert_eq!(
            held.as_ref()
                .map(|(_, frames)| frames.iter().map(|(pilot, _)| *pilot).collect::<Vec<_>>()),
            Some(vec![p0])
        );
        forget_held_pilot(&mut held, p0);
        assert!(held.is_none());
        forget_held_pilot(&mut held, p0);
        assert!(held.is_none());
    }

    /// **16b cut I.3**: a `TempoFloor` is refused at the reserve exactly as an
    /// intent is, and a control, a stop and an ack still fit behind both.
    #[test]
    fn i3_offer_holds_a_tempo_floor_to_the_intent_reserve() {
        let (inbox, _rx) = mpsc::channel(5);
        let (stop, _stop_rx) = watch::channel(false);
        let handle = ArenaHandle {
            match_id: MatchId(7),
            inbox,
            reserve: 3,
            stop,
            reported: Arc::new(AtomicBool::new(false)),
        };
        assert!(
            handle
                .offer(ArenaMsg::TempoFloor {
                    period_ms: Some(5_000)
                })
                .is_ok(),
            "room above the reserve"
        );
        assert!(
            handle
                .offer(ArenaMsg::Intent(order(RED, IntentKind::Startup, 1)))
                .is_ok()
        );
        // Three slots left, all reserve: neither an intent nor a floor —
        // nor, 16b cut I.4, a watch.
        assert!(matches!(
            handle.offer(ArenaMsg::TempoFloor { period_ms: None }),
            Err(ArenaMsg::TempoFloor { period_ms: None })
        ));
        assert!(matches!(
            handle.offer(ArenaMsg::Watch { on: true }),
            Err(ArenaMsg::Watch { on: true })
        ));
        assert!(
            handle
                .offer(ArenaMsg::Intent(order(RED, IntentKind::Startup, 2)))
                .is_err()
        );
        assert!(
            handle
                .offer(ArenaMsg::Control(control(2, ControlState::Live, None)))
                .is_ok()
        );
        assert!(handle.offer(ArenaMsg::Stop(StopReason::Abort)).is_ok());
        assert!(
            handle
                .offer(ArenaMsg::LedgerAck { upto_chunk_seq: 0 })
                .is_ok()
        );
    }

    /// **16b cut I.3**: a staff `Place` is buffered past the per-unit bound and
    /// never refused, and it takes none of the bound a pilot's order meets.
    #[test]
    fn i3_a_place_is_kept_past_the_bound_and_counts_against_nobody() {
        let mut waiting = PrePulse::default();
        let place = |seq| order(RED, IntentKind::Place { x: 3, y: 4 }, seq);
        // The pilot's two of a bound of two.
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(RED, IntentKind::Startup, 1),
            2,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(RED, IntentKind::Startup, 2),
            2,
            MatchId(1)
        ));
        // A place past the bound is kept.
        assert!(buffer_discrete(
            &mut waiting.discrete,
            place(3),
            2,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            place(4),
            2,
            MatchId(1)
        ));
        // And the pilot's third is refused by the pilot's own count, not by the
        // places: a unit whose buffer holds only places still takes the pilot's
        // orders up to the bound.
        assert!(!buffer_discrete(
            &mut waiting.discrete,
            order(RED, IntentKind::Startup, 5),
            2,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(BLUE, IntentKind::Place { x: 1, y: 1 }, 6),
            1,
            MatchId(1)
        ));
        assert!(buffer_discrete(
            &mut waiting.discrete,
            order(BLUE, IntentKind::Startup, 7),
            1,
            MatchId(1)
        ));
        let (_, intents) = waiting.take();
        assert_eq!(
            intents
                .iter()
                .map(|intent| (intent.unit, intent.kind.as_str(), intent.seq))
                .collect::<Vec<_>>(),
            vec![
                (RED, "startup", 1),
                (RED, "startup", 2),
                (RED, "place", 3),
                (RED, "place", 4),
                (BLUE, "place", 6),
                (BLUE, "startup", 7),
            ]
        );
    }

    /// **16b cut I.4**: a `Watch { on: true }` sets `watched` and clears the
    /// picture's second, so the next pulse builds one for the new watcher; a
    /// `Watch { on: false }` clears `watched` and leaves the second alone.
    #[test]
    fn i4_a_watch_clears_the_pictures_second() {
        let mut arena = arena_at_epoch(vec![tank(RED, Team::Red, pose(1_500, 1_500))], 1);
        arena.last_observer_s = Some(4);
        apply_watch(&mut arena, true);
        assert!(arena.watched);
        assert_eq!(arena.last_observer_s, None);
        arena.last_observer_s = Some(5);
        apply_watch(&mut arena, false);
        assert!(!arena.watched);
        assert_eq!(arena.last_observer_s, Some(5));
        assert_eq!(newer_observer(None, None), None);
    }

    /// **16b cut I.4**: a picture a full egress refused is held, a newer
    /// picture replaces it (the older is never sent), a pulse with no picture
    /// of its own sends the held one — even with no frame and inside a second
    /// the clock already carried — and `Task::end` sends a picture still held
    /// before `Ended`.
    #[tokio::test]
    async fn i4_the_held_picture_is_replaced_by_the_newer_and_sent_at_the_end() {
        let arena = arena_at_epoch(vec![tank(RED, Team::Red, pose(1_500, 1_500))], 1);
        let (egress, mut world) = mpsc::channel(1);
        let (_inbox_tx, inbox) = mpsc::channel(4);
        let (_stop_tx, stop) = watch::channel(false);
        let mut task = Task {
            match_id: MatchId(9),
            arena,
            inbox,
            egress,
            stop,
            backlog: VecDeque::new(),
            backlog_bound: 8,
            per_unit_bound: 4,
            stall: Duration::from_secs(60),
            last_clock_s: None,
            held_frames: None,
            held_observer: None,
            held_notice_drops: 0,
            reported: Arc::new(AtomicBool::new(false)),
        };
        let picture = |now_ms: i64| {
            let mut observer = task_picture();
            observer.now_ms = now_ms;
            observer
        };
        let pulse = |observer: Option<Observer>| StepOutput {
            ledger_chunks: Vec::new(),
            frames: Vec::new(),
            tempo: crate::arena::clock::Tempo::Quiet,
            next_due_ms: 0,
            lagged: false,
            ended: None,
            substeps: 0,
            perception: crate::arena::perception::PerceptionCounters::default(),
            reflex: crate::arena::reflex::ReflexCounters::default(),
            deliberation: Vec::new(),
            observer,
        };
        // The first message (the clock of second 0) fills the egress.
        task.publish(pulse(None)).await;
        // A picture now finds it full: held.
        task.publish(pulse(Some(picture(1_000)))).await;
        assert_eq!(task.held_observer.as_ref().map(|o| o.now_ms), Some(1_000));
        // A newer one, still full: the newer is held and the older gone.
        task.publish(pulse(Some(picture(2_000)))).await;
        assert_eq!(task.held_observer.as_ref().map(|o| o.now_ms), Some(2_000));
        assert!(task.held_frames.is_none(), "no frame was ever held");
        // The world takes the clock; a pulse with no picture of its own, in a
        // second the clock already carried, sends the held one.
        assert!(matches!(
            world.recv().await,
            Some(ArenaOut::Frames { observer: None, .. })
        ));
        task.publish(pulse(None)).await;
        assert!(task.held_observer.is_none());
        match world.recv().await {
            Some(ArenaOut::Frames {
                frames, observer, ..
            }) => {
                assert!(frames.is_empty());
                assert_eq!(observer.map(|o| o.now_ms), Some(2_000), "the newer");
            }
            other => panic!("the held picture: {other:?}"),
        }
        // Held again, then the end sends it before `Ended`.
        task.publish(pulse(None)).await; // nothing: same second, no picture
        task.publish(pulse(Some(picture(3_000)))).await; // sent, fills
        task.publish(pulse(Some(picture(4_000)))).await; // held
        assert_eq!(task.held_observer.as_ref().map(|o| o.now_ms), Some(4_000));
        let ender = tokio::spawn(async move {
            task.end(MatchEnd {
                reason: EndReason::Abort,
                t_ms: 4_000,
            })
            .await;
        });
        let mut seen: Vec<Option<i64>> = Vec::new();
        loop {
            match world.recv().await {
                Some(ArenaOut::Frames { observer, .. }) => {
                    seen.push(observer.map(|o| o.now_ms));
                }
                Some(ArenaOut::Ended { .. }) | None => break,
                Some(_) => {}
            }
        }
        ender.await.expect("the end joins");
        assert_eq!(seen, vec![Some(3_000), Some(4_000)]);
    }

    /// A one-unit picture, for the held-picture test.
    fn task_picture() -> Observer {
        arena_at_epoch(vec![tank(RED, Team::Red, pose(1_500, 1_500))], 1).observer()
    }

    #[test]
    fn m1_the_inbox_reserve_is_three_a_unit_and_two() {
        assert_eq!(reserve(0), 2);
        assert_eq!(reserve(2), 8);
        assert_eq!(reserve(24), 74);
    }

    /// **16b cut M.2** (16c `B-51`): the timer's floor. An event due "now" — or
    /// an arena that is behind — sleeps one millisecond, never zero: a zero sleep
    /// re-pulses inside the same wall millisecond and integrates nothing.
    ///
    /// **16b cut U.1**: the deadline takes the task's base, so a deadline already
    /// past is the floor and a future one is the base plus it, not now plus the
    /// interval.
    #[test]
    fn m2_a_deadline_due_now_or_in_the_past_is_at_least_one_millisecond_ahead() {
        let base = Instant::now();
        for due in [0_i64, -1_000, i64::MIN] {
            let before = Instant::now();
            let at = deadline(base, due);
            assert!(
                at >= before + Duration::from_millis(1),
                "deadline(base, {due}) slept for nothing"
            );
        }
        // And a future event is the base plus it, exactly.
        let at = deadline(base, 60_000);
        assert_eq!(at, base + Duration::from_millis(60_000));
    }
}
