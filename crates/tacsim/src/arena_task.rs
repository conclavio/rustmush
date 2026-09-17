//! **One running match, as a task** (16b cut T.2, T1; doc 16 §3.1 "The seam is
//! seven messages", "The arena core is a library", "Clock and lag", "Failure").
//!
//! [`crate::arena`] is a library with no clock, no task and no I/O. This is the
//! thin wrapper doc 16 §3.1 describes and the *only* place the arena meets a
//! runtime: it owns one [`Arena`], a bounded inbox, a bounded egress, a timer
//! driven by [`StepOutput::next_due_ms`], a watchdog at `tempo.stall_ms`, the
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
//! task  -> world  ArenaOut::Ledger      one closed chunk of the match's black box
//! task  -> world  ArenaOut::Frames      the pilots whose view changed this pulse
//! task  -> world  ArenaOut::Lagged      the clock or the egress is behind
//! task  -> world  ArenaOut::Ended       the match is over, with its ledger digest
//! ```
//!
//! `CustodyOut` is the argument to [`spawn_arena`] rather than a message — a
//! match is checked out exactly once, at the moment it starts — and `CustodyIn`
//! is cut T.3's: this cut ends a match with [`ArenaOut::Ended`] and T.3 turns
//! that into a check-in.
//!
//! # The timer, and why a message never steps
//!
//! The task sleeps until `next_due_ms` (doc 16 §3.1 phase 5: the earlier of the
//! next period boundary and the next due event). An arriving message **wakes the
//! task early but does not step it**: it is buffered and drained at the next
//! pulse (§3.1 phases 1 and 2). That is what makes a laggy pilot harmless and a
//! fast one no faster — the tempo decides how often `step` runs, and nothing
//! else does. It is also why an abort is honoured within one period rather than
//! instantly; the immediate lever is the stop signal, which ends the task.
//!
//! # Failure
//!
//! `step` runs under [`std::panic::catch_unwind`]. A panic ends the task with
//! [`ArenaOut::Ended`] carrying [`EndReason::Void`], and **nothing the faulted
//! pulse produced leaves** — there is no `StepOutput` to publish, and the task
//! never touches the arena's chunk egress again. A pulse that takes longer than
//! `tempo.stall_ms` is voided by the watchdog after it returns (the arena has no
//! preemption, so a stalled pulse cannot be interrupted, only refused a
//! successor). Both are consistent with the world task's own firewall: nothing a
//! faulted turn touched reaches the store.

use std::collections::VecDeque;
use std::time::Duration;

use mush_core::ObjId;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::arena::intents::{Control, Intent};
use crate::arena::ledger::LedgerChunk;
use crate::arena::state::{Arena, ArenaError, CustodyOut, EndReason, MatchEnd, StopReason};
use crate::arena::step::{Frame, StepOutput};
use crate::config::{ArenaConfig, MatchConfig};

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
    /// The pilots whose view changed this pulse, in pilot-id order. Latest-wins
    /// per pilot inside the arena, so a slow world drops stale frames and never
    /// events.
    Frames {
        /// Which match.
        match_id: MatchId,
        /// One frame per pilot whose view changed.
        frames: Vec<(ObjId, Frame)>,
    },
    /// The clock absorbed more wall time than the clamp allows, or the egress
    /// is backing up past half of `ledger.max_unflushed_chunks`.
    Lagged {
        /// Which match.
        match_id: MatchId,
    },
    /// The match is over. The last chunks precede this message.
    Ended {
        /// Which match.
        match_id: MatchId,
        /// Why, and when in arena time.
        end: MatchEnd,
        /// The SHA-256 of every ledger byte the match streamed, for the result
        /// record (doc 16 §4).
        digest: [u8; 32],
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
    /// The bounded inbox. Its bound is the world side's own — the arena's per
    /// unit bound (`intents.max_pending_per_unit`) applies again at apply, so a
    /// pilot who floods this queue still cannot queue more than that.
    inbox: mpsc::Sender<ArenaMsg>,
    /// The shutdown tail's lever: `true` ends the task without an
    /// [`ArenaOut::Ended`], because a process that is stopping is not a match
    /// that has a result.
    stop: watch::Sender<bool>,
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
    pub fn offer(&self, msg: ArenaMsg) -> Result<(), ArenaMsg> {
        self.inbox.try_send(msg).map_err(|error| match error {
            mpsc::error::TrySendError::Full(msg) | mpsc::error::TrySendError::Closed(msg) => msg,
        })
    }

    /// Whether the task is still accepting messages.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !self.inbox.is_closed()
    }

    /// Ask the task to end **now**, with no result and no further egress — the
    /// shutdown tail's one lever. Idempotent.
    pub fn signal_stop(&self) {
        let _ = self.stop.send(true);
    }
}

/// **Start one match** (16b T1).
///
/// The arena is built first, on the caller's thread, so a refused check-out
/// ([`ArenaError`]) is the caller's error and not a task that dies on its first
/// poll. Only once it holds an arena does this spawn anything.
///
/// The inbox bound is `intents.max_pending_per_unit × units` — the world side's
/// own bound *before* the arena's, so a burst is refused at the seam rather than
/// filling a queue the arena would then have to purge. The egress `Sender` is
/// shared by every match on the world (one receiver, drained by the world loop).
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
    egress: mpsc::Sender<ArenaOut>,
) -> Result<(ArenaHandle, JoinHandle<()>), ArenaError> {
    let units = custody.units.len();
    let arena = Arena::new(custody, arena_cfg, match_cfg)?;
    let bound = usize::try_from(arena_cfg.intents.max_pending_per_unit)
        .unwrap_or(usize::MAX)
        .saturating_mul(units)
        .max(1);
    let (inbox_tx, inbox_rx) = mpsc::channel(bound);
    let (stop_tx, stop_rx) = watch::channel(false);
    // The pre-pulse buffers get the same bound as the inbox. Without it a pilot
    // who keeps a full inbox full for a whole 5,000 ms `Quiet` period would grow
    // the task's memory without limit: the inbox bounds what is *in flight*, and
    // this bounds what is *waiting for the pulse*.
    let buffer_bound = bound;
    let backlog_bound = usize::try_from(arena_cfg.ledger.max_unflushed_chunks)
        .unwrap_or(usize::MAX)
        .saturating_add(1);
    let stall = Duration::from_millis(u64::from(arena_cfg.tempo.stall_ms));
    let join = runtime.spawn(arena_loop(Task {
        match_id,
        arena,
        inbox: inbox_rx,
        egress,
        stop: stop_rx,
        backlog: VecDeque::new(),
        backlog_bound,
        buffer_bound,
        stall,
    }));
    Ok((
        ArenaHandle {
            match_id,
            inbox: inbox_tx,
            stop: stop_tx,
        },
        join,
    ))
}

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
    /// The bound on each pre-pulse buffer: the same `intents.max_pending_per_unit
    /// × units` the inbox carries.
    buffer_bound: usize,
    /// `tempo.stall_ms`.
    stall: Duration,
}

/// Add one message to a pre-pulse buffer, dropping the **oldest** when the buffer
/// is full.
///
/// Oldest, not newest, because both kinds of message are latest-wins where it
/// matters: a continuous order's newest value is the one the pilot meant, and an
/// epoch never lowers, so the newest `Control` is the one that decides authority.
/// A drop is a flood the world side has already been refusing at the inbox, so it
/// is logged at `warn` and the pulse carries on.
fn push_bounded<T>(buffer: &mut Vec<T>, item: T, bound: usize, what: &str, match_id: MatchId) {
    if buffer.len() >= bound.max(1) {
        buffer.remove(0);
        tracing::warn!(
            %match_id,
            what,
            bound,
            "an arena's pre-pulse buffer is full; the oldest message was dropped"
        );
    }
    buffer.push(item);
}

/// What one pulse asked the loop to do next.
enum Pulse {
    /// Sleep until this deadline and pulse again.
    Again(Instant),
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
    // Every intent and control the world has offered since the last pulse, in
    // arrival order. Cleared by each pulse, so a queue cannot outlive the pulse
    // that drains it.
    let mut controls: Vec<Control> = Vec::new();
    let mut intents: Vec<Intent> = Vec::new();
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
                    Some(ArenaMsg::Control(control)) => push_bounded(
                        &mut controls, control, task.buffer_bound, "control", task.match_id,
                    ),
                    Some(ArenaMsg::Intent(intent)) => push_bounded(
                        &mut intents, intent, task.buffer_bound, "intent", task.match_id,
                    ),
                    // A `Stop` is recorded now and honoured by the next pulse's
                    // phase 1, which is the library's own rule — it never lands
                    // in the middle of a substep.
                    Some(ArenaMsg::Stop(reason)) => task.arena.stop(reason),
                    // An acknowledgement is not an input to `step`: it only
                    // lowers the egress's depth, so it applies on arrival.
                    Some(ArenaMsg::LedgerAck { upto_chunk_seq }) => {
                        task.arena.ledger.mark_flushed(upto_chunk_seq);
                    }
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
                match task.pulse(wall, &controls, &intents).await {
                    Pulse::Again(next) => due = next,
                    Pulse::Done => return,
                }
                controls.clear();
                intents.clear();
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

        if let Some(end) = ended {
            self.end(end).await;
            return Pulse::Done;
        }

        // The watchdog reads the clock *after* `step` returns, because the arena
        // has no preemption: a pulse cannot be cut short, only denied a
        // successor. The stopping pulse below is the library's own `Stop` path,
        // so the `void` event and the ledger's tail are recorded the same way an
        // aborted match records them.
        if took > self.stall {
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

        Pulse::Again(deadline(wall_now_ms, next_due_ms))
    }

    /// Publish one pulse's output: its chunks in order, then its frames, then
    /// the lag notice.
    async fn publish(&mut self, out: StepOutput) {
        let StepOutput {
            ledger_chunks,
            frames,
            lagged,
            ..
        } = out;
        for chunk in ledger_chunks {
            self.queue_chunk(chunk);
        }
        self.drain_backlog();
        if !frames.is_empty() {
            // Frames are the one thing doc 16 §3.1 says a slow world may drop:
            // they are a latest-wins view, and the next pulse's frame supersedes
            // this one.
            let _ = self.egress.try_send(ArenaOut::Frames {
                match_id: self.match_id,
                frames,
            });
        }
        if lagged {
            let _ = self.egress.try_send(ArenaOut::Lagged {
                match_id: self.match_id,
            });
        }
    }

    /// Hand the world the end of the match, waiting for room if it needs some —
    /// but never past the stop signal, which is the process shutting down.
    async fn end(&mut self, end: MatchEnd) {
        self.drain_backlog();
        let message = ArenaOut::Ended {
            match_id: self.match_id,
            end,
            digest: self.arena.ledger.digest(),
        };
        // The one message the task does wait to deliver: a result the world
        // never hears is a match whose units are never checked back in. The loop
        // is for a `watch` that changed without going `true` — that is not a
        // shutdown and must not lose the result.
        loop {
            tokio::select! {
                biased;
                changed = self.stop.changed() => {
                    if changed.is_err() || *self.stop.borrow_and_update() {
                        tracing::warn!(
                            match_id = %self.match_id,
                            "arena task stopped before it could report its result"
                        );
                        break;
                    }
                }
                sent = self.egress.send(message.clone()) => {
                    if sent.is_err() {
                        tracing::warn!(
                            match_id = %self.match_id,
                            "arena result dropped: the world's egress is closed"
                        );
                    }
                    break;
                }
            }
        }
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

/// When to pulse again, from the pulse's own `next_due_ms`.
///
/// Both readings are in the task's monotonic wall milliseconds, so the interval
/// between them is what the timer sleeps — measured from *now* rather than from
/// the base, so a pulse that overran does not ask to be stepped in the past.
/// An arena that is behind gets a zero-length sleep and the next pulse's own
/// clamp (doc 16 §3.1 "Clock and lag") absorbs the lag.
fn deadline(wall_now_ms: i64, next_due_ms: i64) -> Instant {
    let ahead = next_due_ms.saturating_sub(wall_now_ms).max(0);
    Instant::now() + Duration::from_millis(u64::try_from(ahead).unwrap_or(0))
}

/// Milliseconds since the task's monotonic base, saturating.
fn elapsed_ms(base: Instant) -> i64 {
    i64::try_from(base.elapsed().as_millis()).unwrap_or(i64::MAX)
}
