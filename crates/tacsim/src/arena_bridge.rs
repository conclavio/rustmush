//! **The world's half of the arena seam** (16b cut T.2, T3 in part and T4;
//! doc 16 §3.1 "The seam is seven messages", §3.7, §4 "Ledger events", §5).
//!
//! The arena is an authority and the world task is an authority, and they share
//! no state and no lock (D-16-1). What they share is four messages each way, and
//! this module is everything on the world's side of them: the registry of running
//! matches, the routing functions the cockpit verbs call, and the handling of the
//! four messages that come back.
//!
//! # Where each piece lives, and why it is split in two
//!
//! [`crate::arena_bridge::ArenaTasks`] is a **shared handle** — an `Arc` over the table of live tasks,
//! the egress channel and two small queues. It is shared because the two halves
//! that need it do not run in the same place: matches are started and driven from
//! the **world task** (inside a [`mush_core::Ctx`], where every authority check lives), and
//! the tasks are joined by the **process's shutdown tail**, which is outside the
//! world loop entirely (`PersistHandles`). A table that only the world task could
//! see would leave the shutdown tail nothing to join.
//!
//! [`crate::arena_bridge::ArenaRegistry`] is the module state slot, and everything on it is
//! world-task-only: the ledger the chunks are appended to, the epoch mirror
//! intents are stamped from, the world's intent sequence, the panel feed the
//! frames go out on, and one mirror row per match for `@arena/status`. Nothing
//! here is behind a lock, because nothing else ever reads it.
//!
//! # How the egress re-enters the world
//!
//! The arena's egress is one bounded `mpsc` the world loop drains without
//! blocking. Handling a message needs a [`mush_core::Ctx`] — a keyframe is queued on the
//! outbox, a chunk is persisted through the module's own namespace — and the one
//! mechanism in the tree for "run module code with a `Ctx` from the loop" is the
//! typed async re-entry [`mush_core::Effect::ModuleAsync`] /
//! `WorldCommand::ModuleAsyncDone` already uses. So the host does exactly that
//! and no second mechanism: it drains the receiver into [`crate::arena_bridge::ArenaTasks`]'s staging
//! queue and steps the world once with a
//! `ModuleAsyncDone { module: "tacsim", .. }`, whose registered
//! [`mush_core::module::AsyncCompletion`] is [`crate::arena_bridge::ArenaEgress`] below. The messages themselves never go
//! through a serialization: they are moved, not encoded, which is why the staging
//! queue exists at all. Both ends of it run on the world task, so the lock is
//! uncontended.
//!
//! # The ledger, and what "ack-gated" means here
//!
//! Each arena chunk becomes **one** [`mush_core::Ledger`] entry in
//! `log:match.<id>`, carrying the chunk's exact bytes as text. Exact, because
//! doc 16 §4 puts the *arena's* ledger digest on the result record: a reader that
//! concatenates the `lines` fields in chunk order re-derives it byte for byte,
//! which it could not do if the world re-encoded the events.
//!
//! The acknowledgement is the E0 durable-commit high-water and nothing new. The
//! handler records `(match, upto_chunk_seq)` unstamped; the host stamps it with
//! the serial the next batch will take, and sends [`crate::arena_task::ArenaMsg::LedgerAck`] once
//! the writer has published a high-water at or past it. A batch that is dropped
//! or refused therefore never acknowledges, the arena's egress backs up, and the
//! match is voided past `ledger.max_unflushed_chunks` — which is doc 16 §3.1's
//! rule reached by the only path that could reach it honestly.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mush_core::module::{AsyncCompletion, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, ChannelId, CmdError, CommandSpec, Ctx, Ledger, ObjId, PanelFeedHandle,
};
use smol_str::SmolStr;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::arena::clock::Tempo;
use crate::arena::intents::{Control, ControlState, Intent, IntentKind};
use crate::arena::ledger::LedgerEvent;
use crate::arena::state::{Channel, ContactId, MatchEnd, Power, StopReason};
use crate::arena::step::Frame;
use crate::arena_task::{ArenaHandle, ArenaMsg, ArenaOut, MatchId};
use crate::store::{NS, push_op};

/// The panel feed cockpit frames go out on (doc 16 §5). Cut K builds the real
/// cockpit on it; this cut publishes the [`Frame`] the arena library produces.
///
/// **16b cut Q.3** renamed it from `tacsim.frame` to doc 16 §5's own name, which
/// the frame's *schema* id ([`crate::match_::cockpit::FRAME_SCHEMA`]) has
/// carried since cut K.2: the feed was the one place the transport drifted from
/// the document. The schema and its version do not move — every field cut Q adds
/// is additive — and the project is pre-launch, so there is no second lane and
/// no client pinned to the old name.
pub const FRAME_FEED: &str = "arena.cockpit";

/// **The panel feed the arena's terrain goes out on** (16b cut Q.3; doc 16 §3.2,
/// §5).
///
/// It is a **pulled** feed and nothing is ever pushed on it: the only producer
/// is the query handler [`register`] installs, which answers
/// [`crate::match_::cockpit::map_frame`] to a seated pilot and
/// [`crate::match_::cockpit::map_unseated`] to everybody else. A panel client
/// that holds a cockpit frame of match M and no map of M asks for it, and asks
/// again at most once every two seconds until it has one.
///
/// Registered as a feed — rather than only as a query name — because that is
/// what puts `arena.map` in the client manifest and what makes a
/// `PanelSubscribe` route to the handler at all.
///
/// **Its `schema.retain` marker can never fire** (16b cut B.5, `T-62`): a query
/// reply is conn-scoped and never recorded for resume — neither in the session
/// ring nor in the per-feed latest slot the marker selects — so nothing of this
/// feed is ever replayed. The clients' timer heal (16b cut B.2: the web's
/// `MAP_ASK_MS` interval and the TUI pump's tick, under the same two-second
/// latch) is what resumes a map after a reconnect. The marker stays because the
/// payload builders share one schema shape and it costs nothing.
pub const MAP_FEED: &str = "arena.map";

/// The notice category every arena notice carries — every arena line but the
/// per-pulse notice sentences, which carry [`NOTICE_SENTENCE_TAG`].
pub const NOTICE_CATEGORY: &str = "arena";

/// **The tag on the `Sys` copy of a frame's own notice** (16b cut B.1): the
/// sentence [`crate::match_::cockpit::notice_lines`] renders for each notice the
/// bridge is about to push in that frame's `notices` array.
///
/// A panel's notices ring composes those notices from the frame's fields, and
/// since cut B.1 it also rings the pilot's `Sys` replies while seated (on the
/// web's screen 0 it is the only reply surface). Without a tag of its own the
/// sentence would be rung as a reply AND composed from the frame — every notice
/// twice, which is exactly what cut F.2's one owner per sentence forbids. The
/// replies (`Locking BA.`, a usage line) keep [`NOTICE_CATEGORY`]; a ring leaves
/// out a line carrying this tag, and the scrollback prints it as before (no
/// renderer styles either tag). A span tag is on the wire already
/// (`mush_proto::text::Span::tag`), so this is no protocol change.
///
/// **16b cut Y.4**: the catch-up block a reclaim's first frame carries
/// ([`crate::match_::cockpit::catchup_lines`], one `Sys` line of several text
/// lines, said before the frame's notices) carries it too, for the same reason:
/// both rings compose it from the frame's `catchup` object.
pub const NOTICE_SENTENCE_TAG: &str = "arena-notice";

/// The payload the arena-egress re-entry carries. It is a token and not the
/// messages: the messages are moved through [`ArenaTasks::take_staged`].
pub const EGRESS_PAYLOAD: &str = "{\"arena\":\"egress\"}";

/// How many egress messages one re-entry handles. A bound on the world loop's
/// per-iteration cost, not on the arena: whatever is left is handled by the next
/// drain, and the receiver is what holds it meanwhile.
pub const MAX_EGRESS_PER_DRAIN: usize = 256;

/// How deep the staging queue between the host's drain and the handler may get.
/// Both ends run on the world task one after the other, so it is normally
/// `MAX_EGRESS_PER_DRAIN` at most; the bound is what keeps a wedged handler from
/// growing the process.
pub const MAX_STAGED: usize = 4_096;

/// How many matches `@arena/status` renders.
pub const STATUS_PAGE: usize = 32;

/// How long the shutdown tail waits for one arena task to join.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// The shared table
// ---------------------------------------------------------------------------

/// One live match, as the shared table holds it.
struct Live {
    handle: ArenaHandle,
    join: JoinHandle<()>,
}

/// One ledger write waiting to be acknowledged to its arena.
struct PendingAck {
    match_id: MatchId,
    upto_chunk_seq: u64,
    /// The writer serial the batch carrying it will take; `0` until the host
    /// stamps it.
    serial: u64,
}

/// What [`ArenaTasks`] holds behind its lock.
struct Tasks {
    /// Cloned into every [`crate::arena_task::spawn_arena`].
    egress_tx: mpsc::Sender<ArenaOut>,
    /// Taken **once**, by the world loop, at its first iteration.
    egress_rx: Option<mpsc::Receiver<ArenaOut>>,
    live: BTreeMap<MatchId, Live>,
    staged: VecDeque<ArenaOut>,
    acks: Vec<PendingAck>,
}

/// **The table of live arena tasks**, shared by the world task (which starts and
/// drives them) and the process's shutdown tail (which joins them).
///
/// Cheap to clone: it is one `Arc`. The lock is only ever taken by the world task
/// and, once, by the shutdown tail, so it is uncontended by construction — and it
/// is never held across an `await`.
#[derive(Clone)]
pub struct ArenaTasks(Arc<Mutex<Tasks>>);

impl std::fmt::Debug for ArenaTasks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ArenaTasks({} live)", self.live_ids().len())
    }
}

impl ArenaTasks {
    /// A table with a fresh egress of `capacity` messages.
    #[must_use]
    pub fn new(capacity: usize) -> ArenaTasks {
        let (egress_tx, egress_rx) = mpsc::channel(capacity.max(1));
        ArenaTasks(Arc::new(Mutex::new(Tasks {
            egress_tx,
            egress_rx: Some(egress_rx),
            live: BTreeMap::new(),
            staged: VecDeque::new(),
            acks: Vec::new(),
        })))
    }

    /// The egress receiver, once. `None` on every later call — the world loop is
    /// its one owner, and a second caller would silently split the stream.
    #[must_use]
    pub fn take_egress(&self) -> Option<mpsc::Receiver<ArenaOut>> {
        self.0.lock().ok()?.egress_rx.take()
    }

    /// The egress sender every match's task publishes on.
    #[must_use]
    pub fn egress_sender(&self) -> Option<mpsc::Sender<ArenaOut>> {
        Some(self.0.lock().ok()?.egress_tx.clone())
    }

    /// Record one started match. A second insert under the same id replaces the
    /// first and hands it back so the caller can stop and join it rather than
    /// leaking a task.
    pub fn insert(
        &self,
        match_id: MatchId,
        handle: ArenaHandle,
        join: JoinHandle<()>,
    ) -> Option<(ArenaHandle, JoinHandle<()>)> {
        let mut tasks = self.0.lock().ok()?;
        tasks
            .live
            .insert(match_id, Live { handle, join })
            .map(|old| (old.handle, old.join))
    }

    /// Forget one match, handing back its handle and join.
    pub fn remove(&self, match_id: MatchId) -> Option<(ArenaHandle, JoinHandle<()>)> {
        let mut tasks = self.0.lock().ok()?;
        tasks
            .live
            .remove(&match_id)
            .map(|live| (live.handle, live.join))
    }

    /// Every live match, in id order.
    #[must_use]
    pub fn live_ids(&self) -> Vec<MatchId> {
        self.0
            .lock()
            .ok()
            .map(|tasks| tasks.live.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Whether that match's task is still accepting messages.
    #[must_use]
    pub fn is_live(&self, match_id: MatchId) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|tasks| tasks.live.get(&match_id).map(|live| live.handle.is_live()))
            .unwrap_or(false)
    }

    /// Offer one message to that match's task without waiting. `Err(msg)` hands
    /// the message back: no such match, a full inbox, or a task that has ended.
    pub fn offer(&self, match_id: MatchId, msg: ArenaMsg) -> Result<(), ArenaMsg> {
        let Ok(tasks) = self.0.lock() else {
            return Err(msg);
        };
        match tasks.live.get(&match_id) {
            Some(live) => live.handle.offer(msg),
            None => Err(msg),
        }
    }

    /// **Offer one match its `Stop`, and classify a refusal** (16b cut M.1,
    /// `B-49`). Four answers:
    ///
    /// - no such id in the table → [`ControlRefusal::NoMatch`];
    /// - the offer lands → `Ok(())`;
    /// - the offer fails while the task is live (a full inbox) →
    ///   [`ControlRefusal::Busy`];
    /// - the offer fails, the inbox is closed, and the task **has reported**
    ///   ([`ArenaHandle::has_reported`]) → [`ControlRefusal::Busy`]: its `Ended`
    ///   is staged or in the egress, and the world's `drain_egress` will reach
    ///   `on_ended`, which finishes the match with the arena's own units;
    /// - the offer fails, the inbox is closed, and the task has **not** reported
    ///   → [`ControlRefusal::Gone`]: it died without a result.
    ///
    /// # Errors
    ///
    /// As above.
    pub fn send_stop(&self, match_id: MatchId, reason: StopReason) -> Result<(), ControlRefusal> {
        let Ok(tasks) = self.0.lock() else {
            return Err(ControlRefusal::NoMatch);
        };
        let Some(live) = tasks.live.get(&match_id) else {
            return Err(ControlRefusal::NoMatch);
        };
        match live.handle.offer(ArenaMsg::Stop(reason)) {
            Ok(()) => Ok(()),
            Err(_) if live.handle.is_live() || live.handle.has_reported() => {
                Err(ControlRefusal::Busy)
            }
            Err(_) => Err(ControlRefusal::Gone),
        }
    }

    /// Park one drained egress message for the handler. `false` when the staging
    /// queue is full, which is a message the world could not handle and which its
    /// sender therefore never has acknowledged.
    pub fn stage(&self, message: ArenaOut) -> bool {
        let Ok(mut tasks) = self.0.lock() else {
            return false;
        };
        if tasks.staged.len() >= MAX_STAGED {
            return false;
        }
        tasks.staged.push_back(message);
        true
    }

    /// Take up to `max` parked messages, oldest first.
    #[must_use]
    pub fn take_staged(&self, max: usize) -> Vec<ArenaOut> {
        let Ok(mut tasks) = self.0.lock() else {
            return Vec::new();
        };
        let take = max.min(tasks.staged.len());
        tasks.staged.drain(..take).collect()
    }

    /// How many ledger writes are still waiting to be acknowledged to their
    /// arena. Zero once every queued chunk has both reached a batch and been
    /// committed — the observable half of the ack round trip.
    #[must_use]
    pub fn pending_ledger_acks(&self) -> usize {
        self.0.lock().ok().map_or(0, |tasks| tasks.acks.len())
    }

    /// Whether anything is parked.
    #[must_use]
    pub fn has_staged(&self) -> bool {
        self.0
            .lock()
            .ok()
            .is_some_and(|tasks| !tasks.staged.is_empty())
    }

    /// Record that a chunk write has been queued and is waiting for a serial.
    pub fn note_ledger_write(&self, match_id: MatchId, upto_chunk_seq: u64) {
        if let Ok(mut tasks) = self.0.lock() {
            tasks.acks.push(PendingAck {
                match_id,
                upto_chunk_seq,
                serial: 0,
            });
        }
    }

    /// Stamp every unstamped write with the serial its batch will take.
    pub fn stamp_ledger_writes(&self, serial: u64) {
        if let Ok(mut tasks) = self.0.lock() {
            for ack in &mut tasks.acks {
                if ack.serial == 0 {
                    ack.serial = serial;
                }
            }
        }
    }

    /// Acknowledge every stamped write the writer has committed, coalesced to one
    /// message per match at its highest chunk sequence.
    ///
    /// An unstamped write is left alone: it has not reached a batch yet. A write
    /// whose serial the writer never publishes is never acknowledged, which is
    /// the fail-closed half of doc 16 §3.1's bounded egress.
    pub fn ack_ledger_upto(&self, high_water: u64) {
        let ready: BTreeMap<MatchId, u64> = {
            let Ok(mut tasks) = self.0.lock() else {
                return;
            };
            let mut ready: BTreeMap<MatchId, u64> = BTreeMap::new();
            tasks.acks.retain(|ack| {
                if ack.serial == 0 || ack.serial > high_water {
                    return true;
                }
                let entry = ready.entry(ack.match_id).or_default();
                *entry = (*entry).max(ack.upto_chunk_seq);
                false
            });
            ready
        };
        for (match_id, upto_chunk_seq) in ready {
            let _ = self.offer(match_id, ArenaMsg::LedgerAck { upto_chunk_seq });
        }
    }

    /// Ask every live task to end now, with no result — the shutdown tail's one
    /// lever.
    pub fn signal_stop_all(&self) {
        if let Ok(tasks) = self.0.lock() {
            for live in tasks.live.values() {
                live.handle.signal_stop();
            }
        }
    }

    /// **The shutdown tail** (16b T1): set every stop signal, then join every
    /// task under one bound each.
    ///
    /// The lock is taken, drained and released *before* the first await: a
    /// `std::sync::Mutex` guard may not cross one, and a shutdown that deadlocked
    /// on this table would be worse than one that lost a match nobody can finish.
    pub async fn shutdown(&self, timeout: Duration) {
        self.signal_stop_all();
        let joins: Vec<(MatchId, JoinHandle<()>)> = {
            let Ok(mut tasks) = self.0.lock() else {
                return;
            };
            std::mem::take(&mut tasks.live)
                .into_iter()
                .map(|(match_id, live)| (match_id, live.join))
                .collect()
        };
        for (match_id, join) in joins {
            match tokio::time::timeout(timeout, join).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::error!(
                    %match_id,
                    %error,
                    "an arena task did not end cleanly; its match is lost"
                ),
                Err(_) => tracing::error!(
                    %match_id,
                    bound_s = timeout.as_secs(),
                    "an arena task did not join within its bound; shutting down without it"
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The module state slot
// ---------------------------------------------------------------------------

/// What the world remembers about one match, for `@arena/status` and for cut
/// T.3's check-in.
#[derive(Clone, Debug, Default)]
pub struct MatchMirror {
    /// The tempo the last frame reported.
    pub tempo: Tempo,
    /// The highest arena timestamp any ledger event has carried.
    pub arena_ms: i64,
    /// **The arena's own clock** (16b cut D.1), as the last
    /// [`crate::arena_task::ArenaOut::Frames`] reported it, and `end.t_ms` once
    /// the match has ended. Never lowered.
    ///
    /// It is what `SIM STATUS` prints its `arena time m:ss` from.
    /// [`MatchMirror::arena_ms`] is a different number and stays: that is the
    /// last **closed ledger chunk's** timestamp, which is what an operator wants
    /// on `@arena/status` beside the black box's own counts, and it is nought for
    /// the first minutes of a match.
    pub now_ms: i64,
    /// How many `lagged` notices the arena has sent.
    pub lagged: u64,
    /// Chunks appended to `log:match.<id>`.
    pub chunks_appended: u64,
    /// Chunks the ledger refused (a black box that lost tape).
    pub chunks_lost: u64,
    /// Frames delivered to a pilot.
    pub frames_sent: u64,
    /// Set once, when the arena reports the end.
    pub end: Option<MatchEnd>,
}

/// **The last frame one pilot was sent** (16b cut K.2, K3).
///
/// The cockpit's read verbs — `VIEW`, `CONTACTS`, `WEAPONSTATUS`, `STATUS` —
/// render doc 16 §5's frame, and the arena is the only thing that knows any of
/// it. The arena's own [`crate::arena::state::Arena::last_frames`] is inside the
/// task, which a verb cannot reach, so the world keeps the copy it just sent: one
/// row per **pilot**, latest-wins, dropped with the match.
///
/// It is a mirror and never authority. A verb that refused because the mirror is
/// empty is refusing because the arena has not spoken yet, which is the honest
/// answer and the fail-closed one — an unpowered tank is what a pilot who has
/// just boarded has.
#[derive(Clone, Debug)]
pub struct CockpitFrame {
    /// The match the frame came from.
    pub match_id: MatchId,
    /// The arena's own frame.
    pub frame: Frame,
    /// **The range each contact row was published at on this frame**, metres
    /// (16b cut B.4, `T-60`): the memory the next frame's rows are held against
    /// ([`crate::match_::cockpit::held_range_m`]), so a plot near a 50 m edge
    /// does not flip every pulse. Written by `on_frames` through
    /// [`crate::match_::cockpit::settle_ranges`]; every read verb renders the
    /// frame through the `_held` forms with it, so a re-pushed frame carries the
    /// figures the pilot was sent. A fact about what the pilot has been told.
    pub ranges: BTreeMap<ContactId, u64>,
}

/// **The world's registry of running matches** — the tacsim module's own state
/// slot, world-task-only.
#[derive(Clone)]
pub struct ArenaRegistry {
    tasks: ArenaTasks,
    frames: PanelFeedHandle,
    /// The match ledgers. One log per match, `match.<id>`.
    ledger: Ledger,
    /// The world's copy of each arena's `(unit, channel)` epoch table.
    epochs: BTreeMap<(ObjId, Channel), u64>,
    /// The world's monotonic intent sequence, so a queue's order is the pilot's
    /// order and not the inbox's.
    next_seq: u64,
    /// Which match holds a unit (cut T.3's check-out binds it).
    units: BTreeMap<ObjId, MatchId>,
    /// Which match and unit a pilot is seated on (cut T.3's check-out binds it).
    pilots: BTreeMap<ObjId, (MatchId, ObjId)>,
    matches: BTreeMap<MatchId, MatchMirror>,
    /// **16b cut K.2**: the last frame each pilot was sent (see
    /// [`CockpitFrame`]).
    last_frames: BTreeMap<ObjId, CockpitFrame>,
    /// **The one optimistic fact this mirror holds** (16b cut D.1): a pilot whose
    /// `STARTUP` the world has forwarded but whose arena has not answered yet,
    /// and the **world second** the optimism runs out at.
    ///
    /// Everything else here is a copy of something the arena said (D-16-1: the
    /// arena is the single writer while a match runs). This one row is the world
    /// saying *I have just sent that order*, and it exists because a frame at the
    /// `Quiet` tempo is up to five seconds away — so without it `STARTUP`
    /// followed by `HEADING` answers `Power not online: STARTUP first.`, which is
    /// advice to type the thing the pilot has just typed.
    ///
    /// **It ends four ways, and only these four:**
    /// 1. **confirmed** — [`on_frames`] removes it when a frame for that pilot
    ///    carries `power != Off`;
    /// 2. **refused** — [`on_frames`] removes it when a frame for that pilot
    ///    carries a [`crate::arena::perception::FrameNotice::Refused`] whose
    ///    `order` is `IntentKind::Startup.as_str()`. That frame is the clearing
    ///    frame: without this arm a refused `STARTUP` would read
    ///    `startup refused: …` and then `Power is still coming up; hold on.` for
    ///    the rest of the hint;
    /// 3. **expired** — the world clock passes the stamped second, which is a
    ///    `STARTUP` that was lost; the pilot is told to `STARTUP` again, which is
    ///    then true;
    /// 4. **unbound** — [`ArenaRegistry::unbind_pilot`] and
    ///    [`ArenaRegistry::untrack`] drop it wherever they drop
    ///    [`ArenaRegistry::last_frames`].
    ///
    /// A frame carrying `power == Off` and **no** such refusal does *not* clear
    /// it: a batch staged before the intent arrived is drained after it.
    starting_hints: BTreeMap<ObjId, i64>,
    /// **16b cut K.2**: the ground each running match is fought over, so `VIEW`
    /// can draw the window without re-deriving a 65,536-cell arena from the
    /// atlas on every look. One `Arc` per match, taken at check-out and dropped
    /// with the match.
    arenas: BTreeMap<MatchId, Arc<crate::arena::state::ArenaDefinition>>,
    /// **16b cut N.2**: what each running match's squads have been told and when
    /// each order runs out, in **world** milliseconds.
    ///
    /// The arena keeps the authoritative expiry in arena time and journals
    /// `order_expired`; this is the world's own copy, and it exists for exactly one
    /// thing — the in-world sentence doc 16 §6 asks for, which only the world can
    /// say because only the world has an output sink.
    deliberation: BTreeMap<MatchId, MatchDeliberation>,
    /// **The order rate's window** (16b cut Y.2; doc 16 §2 W8's human
    /// trigger): per issuer — the pilot's own tank, `Seat::unit` — the **world**
    /// millisecond of every `ORDER` it gave in the last sixty world seconds,
    /// oldest first. Written and read only by [`ArenaRegistry::note_order_stamp`]
    /// at the world, where `ORDER` is admitted; nothing here crosses the seam,
    /// so the rate can refuse an order and never change an outcome. Cleared per
    /// unit by [`ArenaRegistry::untrack`].
    order_stamps: BTreeMap<ObjId, VecDeque<i64>>,
}

/// **One match's deliberation state, on the world side** (16b cut N.2).
#[derive(Clone, Debug, Default)]
pub struct MatchDeliberation {
    /// Requests deferred and not yet completed. Bounded by
    /// [`crate::deliberation::MAX_IN_FLIGHT_PER_MATCH`] so a match whose provider
    /// has stopped answering cannot queue one per pulse for ever.
    in_flight: usize,
    /// Per squad: when its order runs out in world time, and what it said.
    orders: BTreeMap<crate::deliberation::SquadId, OrderWatch>,
    /// Orders issued, for `@arena/status` and the cost run.
    issued: u64,
    /// **Orders pilots gave** (16b cut Y.1): one per squad an `ORDER` reached,
    /// counted by [`ArenaRegistry::note_human_order`]; `@arena/status`'s
    /// `orders:` line prints it beside [`MatchDeliberation::issued`].
    human_issued: u64,
    /// Requests refused by the provider path, whatever the reason.
    refused: u64,
}

impl MatchDeliberation {
    /// **Whether a pilot's order still stands on `squad`** at world millisecond
    /// `now_ms` (16b cut Y.1; 16c N3's default, *a human order outranks a live LLM
    /// order*): the squad's watch has an issuer and has not expired.
    ///
    /// Read by both halves of the shield — [`on_deliberate`] spends no signal on
    /// such a squad and [`on_deliberation_reply`] drops an order for it — so a
    /// human's order stands until it runs out or the pilot replaces it, and the
    /// machine's commander asks again at the next signal after the expiry.
    #[must_use]
    pub(crate) fn human_holds(&self, squad: crate::deliberation::SquadId, now_ms: i64) -> bool {
        self.orders
            .get(&squad)
            .is_some_and(|watch| watch.issuer.is_some() && watch.expires_ms > now_ms)
    }
}

/// One squad's order, as the world remembers it.
#[derive(Clone, Debug)]
struct OrderWatch {
    /// The world millisecond the order stops standing at.
    expires_ms: i64,
    /// The objective, by its word, for the sentence.
    objective: SmolStr,
    /// Whether the expiry has already been said. A notice is said once.
    said: bool,
    /// **Who gave it** (16b cut Y.1): the pilot's own tank for an `ORDER`, `None`
    /// for the LLM seam. What `MatchDeliberation::human_holds` reads, and what
    /// cut Y.2's span counts per issuer.
    issuer: Option<ObjId>,
}

impl ArenaRegistry {
    /// A registry over `tasks`, publishing frames on `frames`.
    #[must_use]
    pub fn new(tasks: ArenaTasks, frames: PanelFeedHandle) -> ArenaRegistry {
        ArenaRegistry {
            tasks,
            frames,
            ledger: Ledger::new(),
            epochs: BTreeMap::new(),
            next_seq: 0,
            units: BTreeMap::new(),
            pilots: BTreeMap::new(),
            matches: BTreeMap::new(),
            last_frames: BTreeMap::new(),
            starting_hints: BTreeMap::new(),
            arenas: BTreeMap::new(),
            deliberation: BTreeMap::new(),
            order_stamps: BTreeMap::new(),
        }
    }

    /// **16b cut N.2**: how many orders this match has issued and how many
    /// requests were refused — one line of `@arena/status` and the cost run's own
    /// cross-check.
    #[must_use]
    pub fn deliberation_counts(&self, match_id: MatchId) -> (u64, u64, usize) {
        self.deliberation.get(&match_id).map_or((0, 0, 0), |state| {
            (state.issued, state.refused, state.in_flight)
        })
    }

    /// **Remember a pilot's order on `squad`** (16b cut Y.1): the world's watch
    /// on it, latest-wins over whatever was there — an LLM watch included, so
    /// `say_expired_orders` still says its one sentence once per squad — and one
    /// more on the match's `human_issued`.
    ///
    /// `expires_ms` is a **world** millisecond. A plain registry write, `pub` so
    /// an integration test can seed a watch; the cockpit's `ORDER` calls it once
    /// per squad it reached.
    pub fn note_human_order(
        &mut self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
        objective: SmolStr,
        issuer: ObjId,
        expires_ms: i64,
    ) {
        let state = self.deliberation.entry(match_id).or_default();
        state.orders.insert(
            squad,
            OrderWatch {
                expires_ms,
                objective,
                said: false,
                issuer: Some(issuer),
            },
        );
        state.human_issued = state.human_issued.saturating_add(1);
    }

    /// **Who gave the order the world remembers on `squad`** (16b cut Y.1): the
    /// pilot's own tank for an `ORDER`, and `None` for an order from the LLM
    /// seam or a squad with no watch at all. Expired or not: it is the watch's
    /// record, and `MatchDeliberation::human_holds` is what asks whether it
    /// still stands. A read for tests and for cut Y.2's span.
    #[must_use]
    pub fn order_issuer(
        &self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
    ) -> Option<ObjId> {
        self.deliberation
            .get(&match_id)
            .and_then(|state| state.orders.get(&squad))
            .and_then(|watch| watch.issuer)
    }

    /// **The squads `issuer` holds a live order on** (16b cut Y.2; doc 16 §2
    /// W8's span of control): every squad of `match_id` whose world watch names
    /// `issuer` and has not run out at world millisecond `now_ms`.
    ///
    /// The watch is the world's own, in world time, and it runs its full time to
    /// live unless the pilot hands the squad back: the world never learns that
    /// the arena ended an order early (16b cut AA.1, `B-41`), and **16b cut Z.2**
    /// (16c Q44) gave `ORDER` a `RELEASE` whose
    /// [`ArenaRegistry::note_order_released`] drops the issuer's own watch at
    /// once. So the span this answers holds until each order runs out or is
    /// released by the pilot who gave it.
    #[must_use]
    pub fn live_orders_of(
        &self,
        match_id: MatchId,
        issuer: ObjId,
        now_ms: i64,
    ) -> std::collections::BTreeSet<crate::deliberation::SquadId> {
        self.deliberation
            .get(&match_id)
            .map(|state| {
                state
                    .orders
                    .iter()
                    .filter(|(_, watch)| watch.issuer == Some(issuer) && watch.expires_ms > now_ms)
                    .map(|(squad, _)| *squad)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// **A pilot handed `squad` back to doctrine** (16b cut Z.2; 16c Q44): the
    /// world's watch on it is removed **only when its issuer is `releaser`** —
    /// the tank whose pilot typed `RELEASE`.
    ///
    /// The watch is per squad with one issuer, and a non-lead's `RELEASE` is a
    /// `Command::Wingman` to the one machine of their pod pair; an unconditional
    /// removal would free the lead's span and drop the shield
    /// (`MatchDeliberation::human_holds`) while the lead's order still ran on the
    /// rest of the squad. Removed, [`ArenaRegistry::live_orders_of`] frees the
    /// squad at once, `say_expired_orders` never speaks for it (the pilot ended
    /// it; nothing ran out), and the machines' own commander may order it again
    /// at its next signal. `human_issued` is a count of orders given and is left
    /// alone. A plain registry write, `pub` for the integration tests.
    pub fn note_order_released(
        &mut self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
        releaser: ObjId,
    ) {
        if let Some(state) = self.deliberation.get_mut(&match_id)
            && state
                .orders
                .get(&squad)
                .is_some_and(|watch| watch.issuer == Some(releaser))
        {
            state.orders.remove(&squad);
        }
    }

    /// **Spend one of `issuer`'s orders for this minute** (16b cut Y.2; doc 16
    /// §2 W8's human trigger), at world millisecond `now_ms`.
    ///
    /// The stamps older than sixty world seconds are dropped first; if
    /// `per_minute` or more remain, the answer is `Err(count)` and nothing is
    /// written; otherwise the stamp is kept and the answer is `Ok(())`. One
    /// `ORDER ALL` is one stamp, and a stamp is spent before the fan-out, so an
    /// order no unit could be offered still spent it: the pilot typed it.
    ///
    /// # Errors
    ///
    /// `Err(n)`: `issuer` has given `n` orders in the last minute, which is the
    /// ceiling or over it.
    pub fn note_order_stamp(
        &mut self,
        issuer: ObjId,
        now_ms: i64,
        per_minute: u32,
    ) -> Result<(), u32> {
        let stamps = self.order_stamps.entry(issuer).or_default();
        let oldest = now_ms.saturating_sub(60_000);
        while stamps.front().is_some_and(|stamp| *stamp < oldest) {
            stamps.pop_front();
        }
        let held = u32::try_from(stamps.len()).unwrap_or(u32::MAX);
        if held >= per_minute {
            return Err(held);
        }
        stamps.push_back(now_ms);
        Ok(())
    }

    /// The shared table of live tasks.
    #[must_use]
    pub fn tasks(&self) -> &ArenaTasks {
        &self.tasks
    }

    /// Start tracking a match. Cut T.3's check-out calls it with the same units,
    /// pilots and epochs it puts in the `CustodyOut`.
    pub fn track(&mut self, match_id: MatchId) {
        self.matches.entry(match_id).or_default();
    }

    /// Stop tracking a match, handing back its mirror. Cut T.3's check-in calls
    /// it once the units are back.
    ///
    /// The match's units, pilots and epoch rows go with it: an epoch left behind
    /// would be the opening epoch of the *next* match that checked the same unit
    /// out, and an intent stamped from it would authorise against a table that
    /// starts again at whatever the new `CustodyOut` said.
    pub fn untrack(&mut self, match_id: MatchId) -> Option<MatchMirror> {
        let units: Vec<ObjId> = self
            .units
            .iter()
            .filter(|(_, held)| **held == match_id)
            .map(|(unit, _)| *unit)
            .collect();
        // **16b cut D.1**: read off the roster BEFORE it is emptied, because a
        // pilot who typed `STARTUP` and never got a frame has a hint and no
        // remembered frame at all.
        let pilots: Vec<ObjId> = self
            .pilots
            .iter()
            .filter(|(_, (held, _))| *held == match_id)
            .map(|(pilot, _)| *pilot)
            .collect();
        self.units.retain(|_, held| *held != match_id);
        self.pilots.retain(|_, (held, _)| *held != match_id);
        self.epochs.retain(|(unit, _), _| !units.contains(unit));
        // 16b cut K.2: the cockpit's two mirrors go with the match. A frame left
        // behind would be the first thing the *next* match's pilot read.
        self.last_frames.retain(|_, held| held.match_id != match_id);
        // **16b cut D.1**, the hint's fourth end: it goes wherever the frame it
        // is an optimistic stand-in for goes. A hint left behind would tell the
        // *next* match's pilot that their cold tank is coming up.
        for pilot in pilots {
            self.starting_hints.remove(&pilot);
        }
        self.arenas.remove(&match_id);
        // 16b cut N.2: and the deliberation row, for the same reason. An order
        // whose time to live outlives the match it was written for would otherwise
        // say "mind link dropped" about a squad that no longer exists, and an
        // in-flight count left behind would be a permanent debit against the *next*
        // match that took this id.
        self.deliberation.remove(&match_id);
        // **16b cut Y.2**: and the order rate's window for every tank of the
        // match. The issuer is the pilot's own tank, and a tank's next match
        // starts with a commander who has given nothing yet.
        for unit in &units {
            self.order_stamps.remove(unit);
        }
        self.matches.remove(&match_id)
    }

    /// **The last frame that pilot was sent** (16b cut K.2, K3).
    #[must_use]
    pub fn last_frame(&self, pilot: ObjId) -> Option<&CockpitFrame> {
        self.last_frames.get(&pilot)
    }

    /// The ground one running match is fought over (16b cut K.2, K3).
    #[must_use]
    pub fn arena_of(&self, match_id: MatchId) -> Option<&crate::arena::state::ArenaDefinition> {
        self.arenas.get(&match_id).map(Arc::as_ref)
    }

    /// One match's mirror.
    #[must_use]
    pub fn mirror(&self, match_id: MatchId) -> Option<&MatchMirror> {
        self.matches.get(&match_id)
    }

    /// Every tracked match, in id order.
    pub fn tracked(&self) -> impl Iterator<Item = (MatchId, &MatchMirror)> {
        self.matches.iter().map(|(id, mirror)| (*id, mirror))
    }

    /// Bind a unit to the match that holds it (cut T.3's check-out).
    pub fn bind_unit(&mut self, unit: ObjId, match_id: MatchId) {
        self.units.insert(unit, match_id);
    }

    /// Which match holds that unit.
    #[must_use]
    pub fn match_of_unit(&self, unit: ObjId) -> Option<MatchId> {
        self.units.get(&unit).copied()
    }

    /// Bind a pilot to the match and unit it is seated on (cut T.3's
    /// check-out). It is what lets a disconnect reach the arena even after the
    /// seat itself has been released.
    pub fn bind_pilot(&mut self, pilot: ObjId, match_id: MatchId, unit: ObjId) {
        self.pilots.insert(pilot, (match_id, unit));
    }

    /// Which match and unit that pilot is seated on.
    #[must_use]
    pub fn seat_of_pilot(&self, pilot: ObjId) -> Option<(MatchId, ObjId)> {
        self.pilots.get(&pilot).copied()
    }

    /// **Everybody still bound to that match** (16b cut Q.3), in object order —
    /// who [`crate::match_::finish`] owes a cleared cockpit to.
    #[must_use]
    pub fn pilots_of(&self, match_id: MatchId) -> Vec<ObjId> {
        self.pilots
            .iter()
            .filter(|(_, (held, _))| *held == match_id)
            .map(|(pilot, _)| *pilot)
            .collect()
    }

    /// **Forget one pilot's seat** (16b cut F.1) — the undo of [`Self::bind_pilot`],
    /// and the remembered frame with it.
    ///
    /// Until this cut the binding was written at check-out and removed only by
    /// [`ArenaRegistry::untrack`], so a pilot who typed `EXIT` in the first minute
    /// of a match stayed bound for the rest of it: a reconnect fired
    /// `Control { Live }` off that stale row and handed the tank back to
    /// `Controller::Pilot` with nobody in the cockpit — no seat, no focus, no
    /// body in the pod, and the reflex withdrawal cancelled. [`send_control`]
    /// calls this on every `Released` that names its pilot, so the binding lasts
    /// exactly as long as the seat does.
    ///
    /// Answers whether a binding went.
    pub fn unbind_pilot(&mut self, pilot: ObjId) -> bool {
        self.last_frames.remove(&pilot);
        // **16b cut D.1**, the hint's fourth end: the optimism is about a seat,
        // and this is where the seat ends.
        self.starting_hints.remove(&pilot);
        self.pilots.remove(&pilot).is_some()
    }

    /// **Stamp the `Starting` hint for one pilot** (16b cut D.1), expiring at the
    /// world second `until_s`. See `ArenaRegistry::starting_hints` for the four
    /// ways it ends.
    pub fn hint_starting(&mut self, pilot: ObjId, until_s: i64) {
        self.starting_hints.insert(pilot, until_s);
    }

    /// **Whether that pilot's `STARTUP` is still outstanding** at the world second
    /// `now_s` (16b cut D.1). An expired row answers `false` and is left where it
    /// is — this is a read, and the next write past it overwrites it.
    #[must_use]
    pub fn starting_hint(&self, pilot: ObjId, now_s: i64) -> bool {
        self.starting_hints
            .get(&pilot)
            .is_some_and(|until_s| now_s <= *until_s)
    }

    /// **Forget one pilot's `Starting` hint** (16b cut D.1) — the confirmed, the
    /// refused and the unbound ends all come through here.
    pub fn clear_starting_hint(&mut self, pilot: ObjId) {
        self.starting_hints.remove(&pilot);
    }

    /// Publish one `(unit, channel)` epoch into the mirror — cut T.3 seeds it at
    /// check-out with the epochs it hands the arena in `CustodyOut::pilots`.
    /// Never lowers, for the same reason the arena's own table never does.
    pub fn set_epoch(&mut self, unit: ObjId, channel: Channel, epoch: u64) {
        let slot = self.epochs.entry((unit, channel)).or_default();
        *slot = (*slot).max(epoch);
    }

    /// The epoch the arena's table holds for `(unit, channel)`, as far as the
    /// world knows.
    #[must_use]
    pub fn epoch_of(&self, unit: ObjId, channel: Channel) -> u64 {
        self.epochs.get(&(unit, channel)).copied().unwrap_or(0)
    }

    /// Raise `(unit, channel)` and answer the new epoch.
    fn bump_epoch(&mut self, unit: ObjId, channel: Channel) -> u64 {
        let slot = self.epochs.entry((unit, channel)).or_default();
        *slot = slot.saturating_add(1);
        *slot
    }

    /// The next intent sequence.
    fn next_seq(&mut self) -> u64 {
        self.next_seq = self.next_seq.saturating_add(1);
        self.next_seq
    }

    /// Stamp one order with the epoch this mirror holds and the world's next
    /// sequence, **without** the focus, grant and lease checks
    /// [`forward_intent`] makes (16b cut T.3).
    ///
    /// The one caller is the staff `@arena/intent` switch, which orders a unit
    /// *as its own seat* rather than as a pilot sitting in it. The arena still
    /// authorises at apply against its own table, so the epoch is the whole of
    /// what makes the order legal there — a staff order under a stale epoch is
    /// refused by the arena exactly as a pilot's would be.
    pub fn stamp_intent(&mut self, unit: ObjId, channel: Channel, kind: IntentKind) -> Intent {
        Intent {
            unit,
            channel,
            epoch: self.epoch_of(unit, channel),
            kind,
            seq: self.next_seq(),
        }
    }
}

// ---------------------------------------------------------------------------
// Opening a match
// ---------------------------------------------------------------------------

/// Why a match could not be opened.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OpenMatchError {
    /// The tacsim layer did not compose.
    #[error("the arena is not available on this server")]
    Unavailable,
    /// There is no async runtime to spawn the task on — the caller is not on the
    /// world task.
    #[error("the arena can only be started from the world task")]
    NoRuntime,
    /// A match with that id is already running.
    #[error("match {0} is already running")]
    Duplicate(MatchId),
    /// The check-out itself was refused.
    #[error("that check-out is not a match: {0}")]
    Arena(#[from] crate::arena::state::ArenaError),
}

/// **Start one match and register it** (16b T1, the world side).
///
/// The authoritative half of a check-out — the `deployed { arena, snapshot }`
/// marker, the admission-site refusals, the frozen world placement — is cut
/// T.3's, and this is the seam T.3 wraps: everything that has to happen *around*
/// the task, in one place, so the lifecycle owner has one call and not six.
///
/// In order: the task is spawned (so a refused check-out leaves nothing
/// registered), it is inserted into the shared table, the match is tracked, every
/// unit is bound to it, and every seated pilot's station raises the world's epoch
/// mirror **by the same rule the arena's own constructor uses** — a station named
/// `helm`, `sensors` or `fire` raises that one channel, and any other station
/// name raises all three (the cockpit seat of doc 16 §3.7). The mirror and the
/// arena's table therefore agree from the first pulse, which is what lets
/// [`forward_intent`] stamp an epoch the arena will authorise.
///
/// **16b cut V.2**: the answer is every unit's callsign, in unit-id order, as
/// [`crate::arena_task::spawn_arena`] read it off the arena it built — the
/// check-out keeps it on `MatchRecord::callsigns` so the result record can name
/// each unit by the designator every viewer called it by.
///
/// # Errors
///
/// [`OpenMatchError`]: no registry, no runtime, a duplicate id, or whatever
/// `Arena::new` refused.
pub fn open_match(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    custody: crate::arena::state::CustodyOut,
    arena_cfg: &crate::config::ArenaConfig,
    match_cfg: &crate::config::MatchConfig,
    sensors_cfg: &crate::config::SensorsConfig,
) -> Result<crate::arena_task::Callsigns, OpenMatchError> {
    let tasks = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks.clone())
        .ok_or(OpenMatchError::Unavailable)?;
    if tasks.live_ids().contains(&match_id) {
        return Err(OpenMatchError::Duplicate(match_id));
    }
    let egress = tasks.egress_sender().ok_or(OpenMatchError::Unavailable)?;
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| OpenMatchError::NoRuntime)?;
    // Read what the registry has to remember before the message is moved into the
    // task.
    let units: Vec<ObjId> = custody.units.iter().map(|unit| unit.id).collect();
    let pilots: Vec<(ObjId, ObjId, SmolStr, u64)> = custody
        .pilots
        .iter()
        .map(|pilot| (pilot.pilot, pilot.unit, pilot.station.clone(), pilot.epoch))
        .collect();
    // 16b cut K.2: one clone of the ground, for `VIEW`. The definition is moved
    // into the task below and the arena is its single owner from then on, so the
    // world keeps its own immutable copy rather than reaching across the seam or
    // rebuilding it from the atlas per look.
    let ground = Arc::new(custody.definition.clone());
    let (handle, join, callsigns) = crate::arena_task::spawn_arena(
        &runtime,
        match_id,
        custody,
        arena_cfg,
        match_cfg,
        sensors_cfg,
        egress,
    )?;
    if let Some((old, old_join)) = tasks.insert(match_id, handle, join) {
        // Cannot happen (the id was checked above), and if it did, the old task
        // is stopped rather than leaked.
        old.signal_stop();
        old_join.abort();
    }
    let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
        return Err(OpenMatchError::Unavailable);
    };
    registry.track(match_id);
    registry.arenas.insert(match_id, ground);
    for unit in units {
        registry.bind_unit(unit, match_id);
    }
    for (pilot, unit, station, epoch) in pilots {
        registry.bind_pilot(pilot, match_id, unit);
        match Channel::parse(&station) {
            Some(channel) => registry.set_epoch(unit, channel, epoch),
            None => {
                for channel in Channel::ALL {
                    registry.set_epoch(unit, channel, epoch);
                }
            }
        }
    }
    Ok(callsigns)
}

// ---------------------------------------------------------------------------
// Routing: the three functions cut K's verbs and cut T.3's lifecycle call
// ---------------------------------------------------------------------------

/// Why the world refused to forward an order — one named word each, so the
/// cockpit never says "no" (doc 16 §3.7).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IntentRefusal {
    /// That unit is not in a running match.
    #[error("that unit is not in a running match")]
    NoMatch,
    /// The actor is not focused on a station of that unit.
    #[error("you are not at a station of that unit")]
    NotFocused,
    /// The actor holds no live station grant on that unit.
    #[error("you hold no station on that unit")]
    NoStation,
    /// The station's grant does not carry the channel the order needs.
    #[error("your station does not carry the {channel} channel")]
    Channel {
        /// The channel the order needed.
        channel: SmolStr,
    },
    /// Somebody else holds that channel's control lease.
    #[error("another operator holds the {channel} channel")]
    LeaseHeld {
        /// The channel the order needed.
        channel: SmolStr,
    },
    /// The control-lease mirror is incomplete, so the answer is unknown and the
    /// order fails closed.
    #[error("that unit's control state is not known yet")]
    LeaseUnknown,
    /// The arena's inbox is full: the pilot is ordering faster than the arena
    /// runs.
    #[error("that unit already has as many orders queued as it may")]
    Busy,
    /// The match ended between the verb and this call.
    #[error("that match has ended")]
    Gone,
    /// The tacsim layer did not compose.
    #[error("the arena is not available on this server")]
    Unavailable,
}

/// Why the world could not deliver an authority or lifecycle message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ControlRefusal {
    /// No such running match.
    #[error("that unit is not in a running match")]
    NoMatch,
    /// The arena's task has ended **without** reporting a result — the stop
    /// signal, or a dropped inbox, neither of which the world does to a match
    /// while it is alive — or (for a `Control`) its inbox would not take it.
    #[error("that match is not accepting messages")]
    Gone,
    /// **The match cannot take a stop this instant, and is still the arena's**
    /// (16b cut M.1, `B-49`): its inbox is full while its task is live, or its
    /// task has already handed the world its `Ended` and that result is still on
    /// its way through the shared egress. Either way the caller must change
    /// nothing — the arena, not the world, will finish the match — and ask again
    /// in a moment. Before this variant both cases were `Gone`, and an abort
    /// then finished the match with no units, restoring every tank from its
    /// check-out snapshot over the match's real damage.
    #[error("that match is busy; try again in a moment")]
    Busy,
    /// The tacsim layer did not compose.
    #[error("the arena is not available on this server")]
    Unavailable,
}

/// **Forward one order to the arena that holds `unit`** (16b T3).
///
/// Every check doc 16 §3.1 asks for at the world, in the order that makes each
/// next one meaningful:
///
/// 1. the unit is in a running match;
/// 2. `actor` — or the NPC it is driving by proxy — is **focused** on a station
///    of that unit (`mush_sim::focus`);
/// 3. that station's live channel grant carries the channel the order requires
///    (the F3 rule: a gunner's grant is not a driver's);
/// 4. the channel's control lease is either vacant or held by that same
///    principal, and the lease mirror is complete — an unknown answer refuses;
/// 5. the order is stamped with the epoch the world published to the arena and
///    with the world's next sequence, and offered to the inbox without waiting.
///
/// Steps 1–4 are `authorise_seat` since **16b cut Y.1**, which `ORDER`'s
/// [`forward_order`] shares; this function's behaviour is byte-identical to what
/// it was before the factoring.
///
/// The arena authorises again **at apply**, against its own table, because the
/// world's answer was true when the order was typed and an order sits in a queue.
///
/// # Errors
///
/// One [`IntentRefusal`] word. Nothing is sent when anything refuses.
pub fn forward_intent(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    unit: ObjId,
    kind: IntentKind,
) -> Result<(), IntentRefusal> {
    let channel = kind.channel_required();
    let match_id = authorise_seat(ctx, actor, unit, channel)?;

    // (5) The stamp, and the offer.
    let (intent, tasks) = {
        let registry = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .ok_or(IntentRefusal::Unavailable)?;
        let intent = Intent {
            unit,
            channel,
            epoch: registry.epoch_of(unit, channel),
            kind,
            seq: registry.next_seq(),
        };
        (intent, registry.tasks.clone())
    };
    match tasks.offer(match_id, ArenaMsg::Intent(intent)) {
        Ok(()) => Ok(()),
        Err(_) if tasks.is_live(match_id) => Err(IntentRefusal::Busy),
        Err(_) => Err(IntentRefusal::Gone),
    }
}

/// **Is `actor` at a seat of `unit` that carries `channel`?** — steps (1)–(4) of
/// [`forward_intent`], factored out by **16b cut Y.1** so `ORDER` can authorise
/// a commander once on their own tank ([`forward_order`]).
///
/// Answers the running match that holds `unit` — what step 1 finds — so the
/// caller stamps and offers into it without a second lookup.
///
/// # Errors
///
/// One [`IntentRefusal`] word, in step order: `Unavailable`/`NoMatch` (1),
/// `NotFocused` (2), `NoStation`/`Channel` (3), `LeaseUnknown`/`LeaseHeld` (4).
pub(crate) fn authorise_seat(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    unit: ObjId,
    channel: Channel,
) -> Result<MatchId, IntentRefusal> {
    let Some(channel_id) = ChannelId::new(channel.as_str()) else {
        // `Channel::as_str` is three canonical lowercase words; this cannot fail
        // on a tree that compiles, and a refusal is the fail-closed answer if it
        // ever did.
        return Err(IntentRefusal::Channel {
            channel: SmolStr::new(channel.as_str()),
        });
    };

    // (1) The running match.
    let match_id = ctx
        .w
        .module_state::<ArenaRegistry>()
        .ok_or(IntentRefusal::Unavailable)?
        .match_of_unit(unit)
        .ok_or(IntentRefusal::NoMatch)?;

    // (2) Focus. A proxy seat's focus is keyed on the NPC being driven, so the
    // driving actor is the subject of every check below.
    let driver = mush_sim::focus::driving_actor(ctx.w, actor);
    let station = ctx
        .w
        .module_state::<mush_sim::focus::FocusState>()
        .and_then(|focus| focus.focus_of_actor(driver))
        .filter(|focus| focus.unit == unit)
        .map(|focus| focus.station.clone())
        .ok_or(IntentRefusal::NotFocused)?;

    // (3) The live grant. Read rather than revalidated against a remembered
    // epoch: this is the issue point, so the grant in force now is the authority.
    let carries = ctx
        .w
        .command_subjects_of(driver)
        .iter()
        .find(|grant| grant.subject == unit && grant.station.as_str() == station.as_str())
        .ok_or(IntentRefusal::NoStation)?
        .channels
        .contains(&channel_id);
    if !carries {
        return Err(IntentRefusal::Channel {
            channel: SmolStr::new(channel.as_str()),
        });
    }

    // (4) The F3 control lease. The mirror is a 1 Hz projection, so an absent row
    // means "vacant" only when the mirror is complete — otherwise the honest
    // answer is "not known", and an order on an unknown authority is refused.
    if !ctx
        .w
        .module_state::<mush_sim::ControlLeaseView>()
        .is_some_and(mush_sim::ControlLeaseView::is_complete)
    {
        return Err(IntentRefusal::LeaseUnknown);
    }
    let seated = mush_core::PrincipalId::Object { id: driver };
    if let Some(holder) = mush_sim::channel_lease_holder(ctx.w, unit, &channel_id)
        && holder != seated
    {
        return Err(IntentRefusal::LeaseHeld {
            channel: SmolStr::new(channel.as_str()),
        });
    }
    Ok(match_id)
}

/// **Forward a pilot's `ORDER` to every unit it names** (16b cut Y.1; doc 16
/// §3.7, §6; 16c N3).
///
/// The seat is authorised **once, on the issuer's own tank** (`own_unit`) and the
/// `helm` channel (`authorise_seat`) — not on each ordered unit, because a
/// commander is focused on their own station and a wingman flown by the machine
/// has no station anybody is focused on; [`forward_intent`]'s step 2 would refuse
/// every squad order ever typed. Then the order is stamped and offered **per
/// unit** with that unit's own `helm` epoch, exactly as [`send_order`] stamps the
/// LLM seam's. The arena authorises each unit again at apply against its own
/// table: a unit a pilot has since boarded refuses `NotReflex` silently, as
/// today.
///
/// Answers how many units were offered; `Ok(0)` when every offer refused (the
/// cockpit turns that into [`IntentRefusal::Busy`]).
///
/// # Errors
///
/// `authorise_seat`'s refusal, and nothing is offered.
pub fn forward_order(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    own_unit: ObjId,
    units: &[ObjId],
    order: crate::deliberation::Order,
) -> Result<usize, IntentRefusal> {
    let match_id = authorise_seat(ctx, actor, own_unit, Channel::Helm)?;
    let mut offered = 0_usize;
    for unit in units {
        if send_order(ctx, match_id, *unit, order.clone()).is_ok() {
            offered = offered.saturating_add(1);
        }
    }
    Ok(offered)
}

/// **Forward a pilot's `RELEASE` to every unit it names** (16b cut Z.2; 16c
/// Q44) — [`forward_order`]'s shape exactly: the seat authorised **once, on the
/// issuer's own tank** and the `helm` channel, then one
/// [`IntentKind::ReleaseOrder`] `{ squad, issuer: Some(own_unit) }` per unit,
/// stamped with that unit's own `helm` epoch through the shape [`send_order`]
/// uses. The arena drops each unit's order only when it was written for
/// `squad`, so a unit under another squad's order, or none, is untouched.
///
/// Answers how many units were offered; `Ok(0)` when every offer refused (the
/// cockpit turns that into [`IntentRefusal::Busy`]).
///
/// # Errors
///
/// `authorise_seat`'s refusal, and nothing is offered.
pub fn forward_release(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    own_unit: ObjId,
    units: &[ObjId],
    squad: crate::deliberation::SquadId,
) -> Result<usize, IntentRefusal> {
    let match_id = authorise_seat(ctx, actor, own_unit, Channel::Helm)?;
    let mut offered = 0_usize;
    for unit in units {
        let kind = IntentKind::ReleaseOrder {
            squad,
            issuer: Some(own_unit),
        };
        if offer_helm(ctx, match_id, *unit, kind).is_ok() {
            offered = offered.saturating_add(1);
        }
    }
    Ok(offered)
}

/// **Publish one authority change to the arena** (doc 16 §3.1's `Control`).
///
/// The world raises its own mirror for `(unit, channel)` and sends the new
/// epoch; the arena raises its table and **purges every queued intent below it**,
/// which is what makes a pilot who types `FIRE` then `EXIT` in one burst fire
/// nothing.
///
/// The mirror is raised **before** the offer and stays raised if the offer
/// fails: a world that could not tell the arena about a seat change must not go
/// on stamping orders with the old epoch. The arena then refuses everything on
/// that channel, which is the fail-closed direction.
///
/// **16b cut F.1**: `pilot` is whose seat changed, when the world knows. A
/// `Released` that names one is the world's statement that this person is out of
/// that cockpit, and it does three things nothing else did: the arena drops them
/// from its frame roster ([`crate::arena::state::Arena::drop_pilot`]), so no
/// further frame or contact notice is built for them; the registry's own
/// `pilot → (match, unit)` binding goes, so a later reconnect of that account
/// fires no `Control { Live }` and takes nothing back from the controller; and
/// the centre's slot is marked `left`
/// (`crate::match_::lobby::slot_left`), so `SIM STATUS` stops calling somebody
/// standing in the team room `seated`. All three are here, and not in `EXIT`,
/// because `EXIT` is only one of the three routes to a `Released` — a `QUIT` and
/// the linkdead grace running out are the others, and they reach a pilot's
/// standing identically. `None` is a control the world cannot attribute and
/// changes none of them.
///
/// **16b cut Y.3**: none of the three is final any more by itself. A drop (the
/// grace, a `QUIT`) is told to the slot before its `Released` arrives here
/// (`SeatFacts::disconnected` → `lobby::slot_dropped`), and `BOARD` in the
/// running match undoes all three for that pilot: `ArenaRegistry::bind_pilot`,
/// `reseat` (the arena re-adds the roster row on the `Live`) and
/// `lobby::slot_reclaimed`. An `EXIT` stays one-way.
///
/// # Errors
///
/// [`ControlRefusal`]: no such match, or a task that will not take it.
pub fn send_control(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    channel: Channel,
    state: ControlState,
    pilot: Option<ObjId>,
) -> Result<(), ControlRefusal> {
    let (match_id, control, tasks, unbound) = {
        let registry = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .ok_or(ControlRefusal::Unavailable)?;
        let match_id = registry
            .match_of_unit(unit)
            .ok_or(ControlRefusal::NoMatch)?;
        let epoch = registry.bump_epoch(unit, channel);
        // The world side of the same sentence, done here so that every route to a
        // released seat — `EXIT`, the lobby's sweep, the linkdead grace running
        // out — unbinds the pilot, and not only the one verb a reviewer thought of.
        let unbound = match (state, pilot) {
            (ControlState::Released, Some(pilot)) => registry.unbind_pilot(pilot),
            _ => false,
        };
        (
            match_id,
            Control {
                unit,
                channel,
                epoch,
                state,
                pilot,
            },
            registry.tasks.clone(),
            unbound,
        )
    };
    // The centre's own half of the same sentence, outside the registry's borrow.
    // Idempotent: three channels release one seat, and the second and third find
    // the slot already marked.
    if state == ControlState::Released
        && let Some(pilot) = pilot
    {
        crate::match_::lobby::slot_left(ctx, pilot);
    }
    // **16b cut Q.3**: and the panel's half. Gated on the *binding* having
    // actually gone rather than on the state word, so three channels releasing
    // one seat send one cleared frame and not three.
    if let (true, Some(pilot)) = (unbound, pilot) {
        clear_cockpit(ctx, pilot);
    }
    tasks
        .offer(match_id, ArenaMsg::Control(control))
        .map_err(|_| ControlRefusal::Gone)
}

/// **Ask one match to stop** (doc 16 §3.1's `Stop`).
///
/// Honoured at the next pulse's phase 1, never mid-substep, so an abort takes
/// effect within one period of the arena's current tempo.
///
/// # Errors
///
/// [`ControlRefusal::Unavailable`] when the layer did not compose, and otherwise
/// [`ArenaTasks::send_stop`]'s answers: `NoMatch`, `Busy` (a full inbox, or an
/// `Ended` already on its way — change nothing and ask again) or `Gone` (the
/// task died without a result).
pub fn send_stop(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    reason: StopReason,
) -> Result<(), ControlRefusal> {
    ctx.w
        .module_state::<ArenaRegistry>()
        .ok_or(ControlRefusal::Unavailable)?
        .tasks
        .send_stop(match_id, reason)
}

/// Put every channel of one pilot's unit at the state the world now knows.
///
/// The one place the lifecycle facts the kernel publishes turn into `Control`s —
/// four of them since cut K.2 taught the kernel to publish linkdead onset and
/// revival (see [`SeatFacts`]). `BOARD` and `EXIT` are cut K.2's verbs and call
/// [`send_control`] directly, on the same world task, right beside
/// `request_seat_activation`.
fn control_all_channels(ctx: &mut Ctx<'_>, unit: ObjId, state: ControlState, pilot: ObjId) {
    for channel in Channel::ALL {
        if let Err(error) = send_control(ctx, unit, channel, state, Some(pilot)) {
            tracing::debug!(%unit, channel = channel.as_str(), %error, "arena control not delivered");
        }
    }
}

/// **Put a reclaimed pilot back in their tank's seat, at the arena** (16b cut
/// Y.3; 16c `N4`; doc 07 §1.3 "Reclaiming control", step 2).
///
/// One `Control { Live }` per channel naming the pilot, each at a **new** epoch
/// — exactly what a revival inside the grace sends (`SeatFacts::revived`), so
/// nothing queued under the dropped line or by the machine's tenure survives
/// the reclaim. The arena, at its next pulse's phase 1, puts the pilot back on
/// its frame roster (`Arena::seat_pilot`) and hands the tank from the reflex
/// controller to `Controller::Pilot`; a `Released` that reaches it after this
/// `Live` is below the epoch it raised and changes nothing (`apply_control`'s
/// stale gate), so the race is settled by the epoch table and not by arrival.
///
/// `BOARD` in a running match calls this after `ArenaRegistry::bind_pilot`, so
/// the frames the arena then sends find a seated pilot.
pub(crate) fn reseat(ctx: &mut Ctx<'_>, unit: ObjId, pilot: ObjId) {
    control_all_channels(ctx, unit, ControlState::Live, pilot);
}

// ---------------------------------------------------------------------------
// The egress: the four messages that come back
// ---------------------------------------------------------------------------

/// The registered [`AsyncCompletion`] the host's arena-egress wake re-enters
/// through. The payload is a token ([`EGRESS_PAYLOAD`]); the messages themselves
/// are taken from the staging queue.
pub struct ArenaEgress;

impl AsyncCompletion for ArenaEgress {
    fn on_complete(&self, ctx: &mut Ctx<'_>, _correlation: u64, payload: &str) {
        // **16b cut N.2**: the module has two async re-entries now and one
        // registered completion, so this routes on the payload. The egress wake is
        // a fixed token ([`EGRESS_PAYLOAD`]) and everything else is a deliberation
        // reply — which is the narrowest test available, because the token is a
        // constant this crate wrote and a reply is a `DeliberationReply` this crate
        // also wrote.
        if payload == EGRESS_PAYLOAD || payload.is_empty() {
            drain_egress(ctx);
        } else {
            on_deliberation_reply(ctx, payload);
        }
    }
}

/// Handle every parked egress message. Called on the world task, with a `Ctx`.
pub fn drain_egress(ctx: &mut Ctx<'_>) {
    let Some(tasks) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks.clone())
    else {
        return;
    };
    for message in tasks.take_staged(MAX_EGRESS_PER_DRAIN) {
        match message {
            ArenaOut::Frames {
                match_id,
                frames,
                now_ms,
            } => on_frames(ctx, match_id, frames, now_ms),
            ArenaOut::Ledger { match_id, chunk } => {
                on_chunk(ctx, match_id, chunk.seq, &chunk.bytes);
            }
            ArenaOut::Lagged { match_id } => on_lagged(ctx, match_id),
            ArenaOut::Deliberate { match_id, signals } => {
                on_deliberate(ctx, match_id, &signals);
            }
            ArenaOut::Ended {
                match_id,
                end,
                digest,
                units,
            } => on_ended(ctx, match_id, end, digest, &units),
        }
    }
    // **16b cut N.2** (doc 16 §6 "Provider failure"): the one in-world sentence, said
    // once per squad whose order ran out unrenewed. It is swept here, on the world's
    // own drain, because the arena has no sink and the notice has to reach people;
    // the arena's `order_expired` ledger line is the record of the same fact, and
    // neither is derived from the other.
    say_expired_orders(ctx);
}

/// One pulse's frames: a keyframe per pilot, on the feed, through the loop's
/// ordinary output path (doc 16 §3.1 — the arena never touches a sink).
///
/// **16b cut D.1**: `now_ms` is the arena's own clock at the end of the step that
/// produced this message, and the message may carry no frame at all — see
/// [`crate::arena_task::ArenaOut::Frames`]. The mirror takes the clock whether or
/// not anybody was told anything, which is what makes the board's `arena time`
/// line move on a match nobody is moving in.
fn on_frames(ctx: &mut Ctx<'_>, match_id: MatchId, frames: Vec<(ObjId, Frame)>, now_ms: i64) {
    let Some(feed) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.frames.clone())
    else {
        return;
    };
    let mut sent = 0_u64;
    let mut tempo = None;
    // 16b cut T.3: the arena's `Ended` carries a reason and a digest and no
    // poses, so the last pose the world saw on this feed is what a check-in
    // re-places the unit at. Recorded for every frame, whether or not its pilot
    // is still connected — the pose is a fact about the unit, not about who was
    // told.
    let mut poses: Vec<(ObjId, crate::match_::PoseNumbers)> = Vec::new();
    for (pilot, frame) in frames {
        poses.push((frame.unit, crate::match_::PoseNumbers::of(&frame.pose)));
        tempo = Some(frame.tempo);
        // A pilot who was destroyed while the pulse ran gets nothing rather than
        // an event addressed to a dead id (treaty ruling 3: re-validate).
        if ctx.w.get(pilot).is_none() {
            continue;
        }
        // **16b cut Q.3**: and neither does one who stopped being a pilot while
        // it ran. `send_control(Released)` unbinds and clears on the world task
        // the moment `EXIT` is typed, while the arena drops the same pilot from
        // its roster at the start of its next pulse — so a batch staged before
        // the release is drained after it. Without this the mirror would be
        // written again and a live cockpit pushed over the cleared one, which
        // (the marker being what it is) would replace the cleared frame in that
        // session's per-feed latest slot — the one cockpit frame a resume
        // replays (16b cut B.5) — and resurrect a seat that no longer exists.
        let seated = ctx
            .w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot))
            .is_some_and(|(held, _)| held == match_id);
        if !seated {
            continue;
        }
        // **16b cut B.4** (`T-60`): each row's range held against the figure
        // the same row was published at on this pilot's previous frame of this
        // match (none on the first, or across matches — contact ids are the
        // match's own), and the settled figures kept for the next.
        let ranges = {
            let empty = BTreeMap::new();
            let previous = ctx
                .w
                .module_state::<ArenaRegistry>()
                .and_then(|registry| registry.last_frame(pilot))
                .filter(|held| held.match_id == match_id)
                .map_or(&empty, |held| &held.ranges);
            crate::match_::cockpit::settle_ranges(&frame, previous)
        };
        // 16b cut K.2: the world's copy first, so a cockpit verb typed in the
        // same batch reads the frame the pilot was just sent, and then the frame
        // itself — doc 16 §5's keyframe, which cut K.2 filled out.
        let payload = crate::match_::cockpit::status_frame_held(match_id, &frame, &ranges);
        // 16b cut S.2: the notices this frame owes (doc 16 §3.6: *notices fire once
        // per transition*), as ordinary output lines on the pilot's own channel.
        // Rendered through `cockpit::notice_line`, which is also what the frame's
        // own `notices` array carries, so the line client and a panel read one
        // sentence.
        let lines = crate::match_::cockpit::notice_lines(&frame);
        // **16b cut Y.4** (doc 07 §1.3 step 5): the catch-up report a reclaim
        // left, as ONE block — `mush-ansi` splits a `Sys` at its line breaks and
        // the web's line is `pre-wrap` — said before the frame's own notices,
        // because it is about the time before them.
        let catchup = frame
            .catchup
            .as_ref()
            .map(|report| crate::match_::cockpit::catchup_lines(frame.callsign, report).join("\n"));
        // **16b cut D.1**: the hint's first two ends, read off this frame before
        // the mirror is written. `power != Off` is the arena confirming the
        // `STARTUP`; a `Refused` naming the same order is the arena clearing it,
        // and that frame is the one the pilot reads the refusal on — so the
        // sentence and the wait must not both be true. A frame carrying `Off` and
        // no such refusal leaves the hint alone: it may be a batch staged before
        // the intent arrived.
        let confirmed = frame.power != Power::Off;
        let refused = frame.notices.iter().any(|notice| {
            matches!(
                notice,
                crate::arena::perception::FrameNotice::Refused { order, .. }
                    if order.as_str() == IntentKind::Startup.as_str()
            )
        });
        if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
            if confirmed || refused {
                registry.clear_starting_hint(pilot);
            }
            // **Stored with the notices cleared.** The mirror is what `CONTACTS`,
            // `VIEW` and `STATUS` re-render, and a notice re-rendered is a notice
            // said twice; the telling happens here, exactly once.
            let mut remembered = frame.clone();
            remembered.notices.clear();
            // 16b cut Y.4: nor the catch-up, for the same reason — `STATUS` and
            // `VIEW` must never re-say it.
            remembered.catchup = None;
            registry.last_frames.insert(
                pilot,
                CockpitFrame {
                    match_id,
                    frame: remembered,
                    ranges,
                },
            );
        }
        // 16b cut B.1: tagged as the frame's own notices, so a panel ring that
        // takes the pilot's replies can leave these out — it composes them from
        // the frame that follows (see `NOTICE_SENTENCE_TAG`). 16b cut Y.4: the
        // catch-up block likewise, first — both rings compose it from the
        // frame's `catchup` object.
        if let Some(block) = catchup {
            ctx.out.sys_to_actor(pilot, NOTICE_SENTENCE_TAG, block);
        }
        for line in lines {
            ctx.out.sys_to_actor(pilot, NOTICE_SENTENCE_TAG, line);
        }
        feed.keyframe_to(ctx, Audience::Actor(pilot), payload);
        sent = sent.saturating_add(1);
    }
    if let Some(record) = ctx
        .w
        .module_state_mut::<crate::match_::MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        for (unit, pose) in poses {
            record.last_pose.insert(unit, pose);
        }
    }
    if let Some(mirror) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.matches.get_mut(&match_id))
    {
        mirror.frames_sent = mirror.frames_sent.saturating_add(sent);
        // **16b cut D.1**: the arena's own clock, whether or not anybody was
        // told anything. `max` rather than a plain write, for the same reason
        // `arena_ms` takes one: the world drains a queue, and a clock that went
        // backwards would be a board that did.
        mirror.now_ms = mirror.now_ms.max(now_ms);
        if let Some(tempo) = tempo {
            mirror.tempo = tempo;
        }
    }
}

/// **Tell one pilot's panel that there is no cockpit any more** (16b cut Q.3).
///
/// One `arena.cockpit` keyframe carrying
/// [`crate::match_::cockpit::cockpit_cleared`], down the same output path the
/// pulse frames take. It prints nothing on a line client — the payload declares
/// no shape — so the ordinary end-of-match and `EXIT` lines a pilot reads are
/// exactly what they were; a panel clears; and because the payload carries the
/// same `schema.retain` marker every frame before it did, it replaces the live
/// frame in that session's per-feed latest slot — the one `arena.cockpit` frame
/// a resume replays, kept beside the ring rather than in it (16b cut B.5) —
/// UNTIL the pilot is seated again, when the next match's first frame replaces
/// it in turn. That is why a panel resets its own notices ring on a change of
/// match and never relies on this frame arriving (16b cut B.2).
///
/// Sent exactly once per pilot, from the two places a pilot stops being one:
/// [`send_control`], when a `Released` actually unbinds somebody, and
/// [`crate::match_::finish`], for anybody still bound when the match is
/// untracked.
pub fn clear_cockpit(ctx: &mut Ctx<'_>, pilot: ObjId) {
    let Some(feed) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.frames.clone())
    else {
        return;
    };
    // A pilot whose object has gone with the match gets nothing rather than an
    // event addressed to a dead id (treaty ruling 3: re-validate), which is the
    // same rule `on_frames` follows.
    if ctx.w.get(pilot).is_none() {
        return;
    }
    feed.keyframe_to(
        ctx,
        Audience::Actor(pilot),
        crate::match_::cockpit::cockpit_cleared(),
    );
}

/// The ledger log id one match streams into: `log:match.<id>` is
/// `chunk_key("match.<id>", n)`, so the log id itself is `match.<id>`.
#[must_use]
pub fn match_log_id(match_id: MatchId) -> String {
    format!("match.{}", match_id.0)
}

/// One closed chunk, appended to `log:match.<id>` on the world's own writer
/// lane (16b T4).
///
/// The chunk's **exact bytes** go in, as one entry's `lines` field: doc 16 §4
/// puts the arena's own ledger digest on the result record, and a reader that
/// concatenates these in chunk order re-derives it. A chunk that is not UTF-8 —
/// which nothing the arena writes can be — is counted as lost and acknowledged
/// anyway, because a black box that lost a page is not a reason to wedge the
/// match that is still running.
fn on_chunk(ctx: &mut Ctx<'_>, match_id: MatchId, seq: u64, bytes: &[u8]) {
    let log_id = match_log_id(match_id);
    let Ok(lines) = std::str::from_utf8(bytes) else {
        tracing::error!(
            %match_id,
            chunk = seq,
            "a match ledger chunk was not UTF-8 and was not recorded"
        );
        // **16b cut V.3**: every debrief fold on the match learns that a page
        // of its tape is gone, and its report says so.
        if let Some(record) = ctx
            .w
            .module_state_mut::<crate::match_::MatchBook>()
            .and_then(|book| book.get_mut(match_id))
        {
            for fold in record.folds.values_mut() {
                fold.lost_chunk();
            }
        }
        lose_chunk(ctx, match_id, seq);
        return;
    };
    // **16b cut V.3**: each line is parsed ONCE, here, and the one list feeds
    // the arena clock, the last pose and every debrief fold. Until this cut the
    // two readers below parsed the chunk once each; they read the list now.
    // This is per chunk and never per pulse: a chunk closes on fill, and the
    // harness's `Arena::step` lens never sees it.
    let mut unparsed = 0_u32;
    let events: Vec<LedgerEvent> = lines
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<LedgerEvent>(line) {
            Ok(event) => Some(event),
            Err(_) => {
                unparsed = unparsed.saturating_add(1);
                None
            }
        })
        .collect();
    let arena_ms = last_event_ms(&events);
    // 16b cut T.3: the black box is also where a check-in's final pose comes
    // from when nobody is in the cockpit. `position_summary` names every unit
    // every `ledger.summary_ms`, so a match with no pilot at all still hands the
    // world the arena's own numbers.
    if let Some(record) = ctx
        .w
        .module_state_mut::<crate::match_::MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        if let Some(poses) = last_position_summary(&events) {
            for (unit, numbers) in poses {
                record.last_pose.insert(unit, numbers);
            }
        }
        // **16b cut V.3 (16c N2, Q37)**: the debrief folds eat the tape as it
        // passes — every event, in line order, to every pilot's fold — so the
        // report is never folded from a read of the store afterwards.
        record.unparsed = record.unparsed.saturating_add(unparsed);
        for fold in record.folds.values_mut() {
            for event in &events {
                fold.absorb(event);
            }
        }
    }
    let value = serde_json::json!({
        "chunk": seq,
        "bytes": bytes.len(),
        "lines": lines,
    });
    let (tasks, op) = {
        let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
            return;
        };
        let prepared = match registry.ledger.prepare_append(&log_id, value) {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::error!(
                    %match_id,
                    chunk = seq,
                    %error,
                    "a match ledger chunk was refused without mutating the ledger"
                );
                if let Some(mirror) = registry.matches.get_mut(&match_id) {
                    mirror.chunks_lost = mirror.chunks_lost.saturating_add(1);
                }
                let tasks = registry.tasks.clone();
                // Acknowledge it anyway: the chunk is lost to the log, and
                // holding the arena's egress open for a page that will never be
                // written would void a match over a record nobody can produce.
                let _ = tasks.offer(
                    match_id,
                    ArenaMsg::LedgerAck {
                        upto_chunk_seq: seq,
                    },
                );
                return;
            }
        };
        match registry.ledger.apply_prepared(prepared) {
            Ok((_entry_seq, op)) => {
                if let Some(mirror) = registry.matches.get_mut(&match_id) {
                    mirror.chunks_appended = mirror.chunks_appended.saturating_add(1);
                    mirror.arena_ms = mirror.arena_ms.max(arena_ms);
                }
                (registry.tasks.clone(), op)
            }
            Err(error) => {
                tracing::error!(
                    %match_id,
                    chunk = seq,
                    %error,
                    "a match ledger append became stale between prepare and apply"
                );
                return;
            }
        }
    };
    // The module's own namespace (`tacsim`), on the ordinary module-op lane —
    // the same path `mush-scenes` and `mush-canon` append their ledgers on, and
    // the one the E0 retained table and its durable-commit high-water sit on.
    push_op(ctx.out, op);
    tasks.note_ledger_write(match_id, seq);
}

/// Count one chunk as lost and acknowledge it, so the arena's bounded egress
/// does not stall on a page that will never be written.
fn lose_chunk(ctx: &mut Ctx<'_>, match_id: MatchId, seq: u64) {
    let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
        return;
    };
    if let Some(mirror) = registry.matches.get_mut(&match_id) {
        mirror.chunks_lost = mirror.chunks_lost.saturating_add(1);
    }
    let tasks = registry.tasks.clone();
    let _ = tasks.offer(
        match_id,
        ArenaMsg::LedgerAck {
            upto_chunk_seq: seq,
        },
    );
}

/// The last `position_summary` in a chunk, as the four integers per unit.
///
/// **16b cut V.3**: over the chunk's parsed events, which `on_chunk` parses once
/// for every reader.
fn last_position_summary(
    events: &[LedgerEvent],
) -> Option<Vec<(ObjId, crate::match_::PoseNumbers)>> {
    events.iter().rev().find_map(|event| match &event.kind {
        crate::arena::ledger::EventKind::PositionSummary { units } => Some(
            units
                .iter()
                .map(|&(unit, x_cm, y_cm, z_cm, heading_mdeg)| {
                    (
                        unit,
                        crate::match_::PoseNumbers {
                            x_cm,
                            y_cm,
                            z_cm,
                            heading_mdeg,
                        },
                    )
                })
                .collect(),
        ),
        _ => None,
    })
}

/// The highest `t_ms` in a chunk, read from its last parsable line, which is
/// what lets `@arena/status` report arena time without the arena reporting it.
///
/// **16b cut V.3**: over the chunk's parsed events, which `on_chunk` parses once
/// for every reader.
fn last_event_ms(events: &[LedgerEvent]) -> i64 {
    events.last().map_or(0, |event| event.t_ms)
}

/// The arena says the clock or the egress is behind: one log line and one staff
/// notice.
fn on_lagged(ctx: &mut Ctx<'_>, match_id: MatchId) {
    let count = {
        let Some(mirror) = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .and_then(|registry| registry.matches.get_mut(&match_id))
        else {
            return;
        };
        mirror.lagged = mirror.lagged.saturating_add(1);
        mirror.lagged
    };
    tracing::warn!(
        %match_id,
        lagged = count,
        "arena match is lagging: the pulse absorbed more wall time than the clamp allows, or the \
         world is not draining its ledger"
    );
    notify_staff(
        ctx,
        format!("Arena match {match_id} is lagging ({count} notices so far)."),
    );
}

/// The arena reports the end. Cut T.2 records it and tells staff; cut T.3 turns
/// this into check-in (the units' cells and final poses through `apply` and
/// `mush_sim::place`, the `deployed` markers cleared, the result record written).
fn on_ended(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    end: MatchEnd,
    digest: [u8; 32],
    units: &[crate::arena::damage::UnitReturn],
) {
    if let Some(mirror) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.matches.get_mut(&match_id))
    {
        mirror.end = Some(end);
        mirror.arena_ms = mirror.arena_ms.max(end.t_ms);
        // **16b cut D.1**: the last reading of the arena's clock is the one the
        // end carries, so a board read after the match stops shows the length of
        // the fight and not the last pulse before it.
        mirror.now_ms = mirror.now_ms.max(end.t_ms);
    }
    let digest = digest_text(&digest);
    tracing::info!(
        %match_id,
        reason = ?end.reason,
        arena_ms = end.t_ms,
        %digest,
        "arena match ended"
    );
    // 16b cut T.3: this is where an end becomes a check-in — the units' cells
    // and final poses back through `apply` and `mush_sim::place`, the markers
    // cleared, the seats released, the match untracked and the result written,
    // all in this one re-entry's authoritative step.
    let checked_in = crate::match_::finish(ctx, match_id, end, digest.clone(), units);
    notify_staff(
        ctx,
        format!(
            "Arena match {match_id} ended: {:?} at {} ms of arena time. Ledger digest {digest}.              {}",
            end.reason,
            end.t_ms,
            if checked_in {
                "Its units are checked in."
            } else {
                "It had no live record; nothing was checked in."
            }
        ),
    );
}

/// The ledger digest, as the 64 lowercase base-16 characters every other digest
/// in the tree renders as.
#[must_use]
pub fn digest_text(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(64);
    for byte in digest {
        // `write!` into a `String` is infallible; the result is discarded rather
        // than unwrapped, which is also what keeps this off the panic budget.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// One line to every connected Wizard-or-better — the one staff audience the
/// tree fans an alert out to.
fn notify_staff(ctx: &mut Ctx<'_>, text: String) {
    let now_ms = ctx.w.now().saturating_mul(1_000);
    for staff in ctx.w.connected_wizards() {
        ctx.out
            .notice_warn_to_actor(staff, now_ms, NOTICE_CATEGORY, text.clone(), None);
    }
}

// ---------------------------------------------------------------------------
// The seat facts this cut can observe
// ---------------------------------------------------------------------------

/// **The four seat lifecycle facts the kernel publishes** (16b cut T.2, widened
/// by cut K.2's K4).
///
/// Cut T.2 could observe two — a full disconnect (`mush-sim`'s own
/// `Disconnected` hook does the seat release; this tells the arena) and a
/// reconnect — and recorded that linkdead **onset** and **revival inside the
/// grace** had no hook at all. Cut K.2 added them to the kernel:
/// [`mush_core::HookPoint::Linkdead`] at the engine's own link-dead announce,
/// and [`mush_core::EventHook::revived`] over `Connected`'s registrations. So
/// doc 16 §3.8's "Disconnects" paragraph is now four lines of code:
///
/// | fact | what the arena is told | what the tank does |
/// |---|---|---|
/// | linkdead onset | `Control { Linkdead }` | keeps its continuous orders, fires nothing |
/// | revival in the grace | `Control { Live }`, new epoch | takes orders again at the next pulse |
/// | the grace expires | `Control { Released }` | zeroes its desired speed and idles |
/// | a clean connect | `Control { Live }` | (a seat that was never released) |
///
/// **16b cut Y.3** added the fifth fact, which is not a hook: past the grace the
/// tank is the reflex controller's and the board reads `flown by doctrine`; a
/// reconnect is told so, and `BOARD` in the running match sends
/// `Control { Live }` at a new epoch through [`reseat`] — the same message a
/// revival inside the grace sends, from the verb rather than the kernel.
///
/// Every one of them goes out as an **epoch raise**, so the arena purges whatever
/// that seat had queued: a pilot whose line drops mid-burst does not fire the
/// shot they had queued a moment before, and a pilot who comes back does not
/// inherit one.
///
/// `EXIT`, `SIM ABORT` and `BOARD` are cut K's verbs, which call
/// [`send_control`] directly beside `request_seat_activation`; lease revocation
/// by a third party cannot happen to a deployed unit at all, because cut T.3's
/// admission-site refusals refuse it.
struct SeatFacts;

impl SeatFacts {
    /// The unit one actor is seated on, as the registry knows it.
    fn seated_unit(ctx: &Ctx<'_>, actor: ObjId) -> Option<ObjId> {
        ctx.w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(actor))
            .map(|(_match_id, unit)| unit)
    }
}

impl mush_core::EventHook for SeatFacts {
    /// A clean connect: a seat that was never released is `Live` again.
    ///
    /// **16b cut Y.3 (16c `N4`)**: a pilot with **no** seat whose slot in the
    /// running match was taken by a drop is told one line — their own tank is
    /// flying on doctrine and `BOARD` takes it back — because before this cut a
    /// reconnecting pilot past the grace was told nothing at all. The tank is
    /// the pilot's own, which they may always be told about (doc 16 §1); what
    /// it did while they were gone is not said here.
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            control_all_channels(ctx, unit, ControlState::Live, actor);
            return;
        }
        let dropped = crate::match_::lobby::centre(ctx.w)
            .filter(|centre| centre.state == crate::match_::MatchState::Running)
            .and_then(|centre| centre.slot_of_pilot(actor))
            .filter(|slot| slot.is_dropped())
            .and_then(|slot| slot.unit);
        if let Some(tank) = dropped {
            let line = format!(
                "{} is flying on doctrine; BOARD takes it back.",
                mush_core::output::display_name(ctx.w, tank)
            );
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        }
    }

    /// **16b cut K.2 (K4)**: the pilot's line came back inside the grace. The
    /// seat is reclaimed with a **new epoch** — `send_control` raises the
    /// world's mirror and the arena raises its table at its next pulse's phase
    /// 1, which is doc 16 §3.8's "at a pulse boundary" exactly — so nothing
    /// queued under the dead line survives the reclaim.
    fn revived(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            control_all_channels(ctx, unit, ControlState::Live, actor);
            // And the pilot is told, because the one thing they cannot see from
            // inside a pod is what happened while they were not there: the tank
            // held the last heading and speed they gave it and fired nothing.
            // It is also the only line that distinguishes a reclaim from an
            // ordinary reconnect, which is what makes the grace observable at
            // all.
            let line = format!(
                "You are back at your station on {}; it held your orders and fired nothing.",
                mush_core::output::display_name(ctx.w, unit)
            );
            ctx.out.sys_to_actor(actor, NOTICE_CATEGORY, line);
        }
    }

    /// **16b cut K.2 (K4)**: the pilot's line dropped and the grace is running.
    /// Not a release: the tank keeps steering and stops shooting.
    fn linkdead(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            control_all_channels(ctx, unit, ControlState::Linkdead, actor);
        }
    }

    /// The grace ran out, or the pilot quit: the seat is released and the tank
    /// goes to the reflex controller.
    ///
    /// **16b cut Y.3 (16c `N4`)**: and it is a **drop**, which the pilot may take
    /// back (`BOARD` in the running match; doc 07 §1.3). A clean `QUIT` reaches
    /// this hook too and is a drop like the grace running out — `EXIT` is the
    /// one one-way route. So the slot is told the route here, through
    /// `lobby::slot_dropped`, and in this order:
    ///
    /// 1. **inside the seated arm** — after an `EXIT` the registry binding is
    ///    gone and this arm never runs, so an `EXIT` followed by a `QUIT` marks
    ///    nothing and the leave stays final;
    /// 2. **before** the `Released`, because [`send_control`] marks the slot
    ///    `left` (`lobby::slot_left`), and `slot_dropped` writes only on a slot
    ///    not yet `left`: the slot has to know the route before it is marked.
    ///
    /// [`send_control`]'s signature and every caller are untouched.
    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            let now_ms = ctx.w.now().saturating_mul(1_000);
            crate::match_::lobby::slot_dropped(ctx, actor, now_ms);
            control_all_channels(ctx, unit, ControlState::Released, actor);
        }
    }
}

// ---------------------------------------------------------------------------
// `@arena/status`
// ---------------------------------------------------------------------------

/// Staff, on `mush-market`'s exact test — the same predicate the rest of this
/// crate's staff surface uses.
fn is_staff(ctx: &Ctx<'_>) -> bool {
    mush_core::perm::has_power(ctx.w, ctx.actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty)
}

/// `@arena/status` — the read (16b cut T.2, extended with cut T.3's lifecycle
/// state).
///
/// # Errors
///
/// [`CmdError`] for a non-staff caller.
pub fn status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !is_staff(ctx) {
        return Err(CmdError::msg("Permission denied."));
    }
    // 16b cut K.1: the simulator centre first, when there is one to see — the
    // same lines `SIM STATUS` renders in the lobby, so staff and pilots read one
    // state and not two. A centre nobody has touched adds nothing, so a quiet
    // server still answers with its one line.
    let mut rows = crate::match_::lobby::staff_status_rows(ctx.w);
    rows.extend(status_rows(ctx));
    let actor = ctx.actor;
    if rows.is_empty() {
        ctx.out.sys_to_actor(actor, "arena", "No match is running.");
    }
    for row in rows {
        ctx.out.sys_to_actor(actor, "arena", row);
    }
    // **16b cut U.4 (16c `T-67`)**: and then every unit a stranded marker still
    // holds, by dbref. Since this cut `@find` omits a unit an arena holds — any
    // marker, live match or not, which is `deployed::excluded`'s rule — and
    // `@arena/release`, the repair for a marker a failed check-in left, takes a
    // dbref and nothing else. This is where staff read it.
    for row in stranded_rows(ctx.w) {
        ctx.out.sys_to_actor(actor, "arena", row);
    }
    Ok(())
}

/// **One line per unit a stranded `deployed` marker holds** (16b cut U.4) —
/// exactly the units `@arena/release` would accept: a marker whose unit no live
/// match of the registry binds, the test that verb refuses by.
///
/// A staff surface, so it names object ids (doc 16 §5's observer rule is about
/// pilots; `crate::match_::verbs`' own head says the same of `@arena/release`).
fn stranded_rows(w: &mush_core::World) -> Vec<String> {
    let registry = w.module_state::<ArenaRegistry>();
    mush_sim::deployed::deployed_units(w)
        .into_iter()
        .filter(|(unit, _)| registry.is_none_or(|registry| registry.match_of_unit(*unit).is_none()))
        .map(|(unit, mark)| {
            format!(
                "#{} carries a stranded deployed marker (match {}); @arena/release #{}",
                unit.0, mark.match_id, unit.0
            )
        })
        .collect()
}

/// One row per running match: its tempo, its arena time, its lag and its chunk
/// counts. A read and nothing else.
fn status_rows(ctx: &Ctx<'_>) -> Vec<String> {
    let Some(registry) = ctx.w.module_state::<ArenaRegistry>() else {
        return vec!["The arena is not available on this server.".to_owned()];
    };
    let live = registry.tasks.live_ids();
    // One number for the whole server rather than per match: the writes are
    // stamped with the batch serial they ride, not with the match they came from,
    // so "how far behind the acknowledgement lane is" is a property of the lane.
    let unacked = registry.tasks.pending_ledger_acks();
    let mut rows = Vec::new();
    for (match_id, mirror) in registry.tracked().take(STATUS_PAGE) {
        // 16b cut T.3: the lifecycle state is the match book's, and the arena's
        // own end reason is what it ended *with*; both are printed, because a
        // match in `stopping` with no end yet is exactly the state an operator
        // needs to be able to see.
        let lifecycle = crate::match_::state_word(ctx.w, match_id);
        let state = match (lifecycle, mirror.end) {
            (Some(word), Some(end)) => format!("{word} ({:?})", end.reason),
            (Some(word), None) => word.to_owned(),
            (None, Some(end)) => format!("{:?}", end.reason),
            (None, None) if live.contains(&match_id) => "running".to_owned(),
            (None, None) => "no task".to_owned(),
        };
        // **16b cut D.1**: the ledger's figure keeps its place — it is what an
        // operator reads the black box by — and the arena's own clock goes beside
        // it in the same `m:ss` the board prints, so the two can be told apart at
        // a glance.
        rows.push(format!(
            "match {match_id}: {state}, tempo {}, arena {} ms, clock {}, lagged {}, chunks {} \
             appended / {} lost, frames {}",
            mirror.tempo.as_str(),
            mirror.arena_ms,
            crate::match_::lobby::clock_text(mirror.now_ms),
            mirror.lagged,
            mirror.chunks_appended,
            mirror.chunks_lost,
            mirror.frames_sent,
        ));
        // **16b cut Y.1**: who has been giving orders — a line of its own and not
        // a clause on the match row, because `mushd`'s test support parses that
        // row (`arena_clock_seconds`).
        let (human, machine) = registry
            .deliberation
            .get(&match_id)
            .map_or((0, 0), |state| (state.human_issued, state.issued));
        rows.push(format!(
            "  orders: {human} by pilots, {machine} by the machines"
        ));
    }
    if !rows.is_empty() {
        rows.push(format!(
            "ledger writes awaiting the store's acknowledgement: {unacked}"
        ));
    }
    if registry.tracked().count() > STATUS_PAGE {
        rows.push(format!(
            "… and more than {STATUS_PAGE} matches; the rest are not shown."
        ));
    }
    rows
}

// ---------------------------------------------------------------------------
// The two queries (16b cut Q.3)
// ---------------------------------------------------------------------------

/// The match and ground one actor is flying in, or `None` for anybody who is
/// not seated on a deployed unit of a tracked match.
///
/// The binding is the registry's own `pilot → (match, unit)` row, which
/// check-out writes and which `EXIT`, a released seat, a linkdead grace running
/// out and `untrack` all take away. So "seated" here means exactly what it means
/// everywhere else in this module, and a pilot who has left reads as a
/// bystander from the next line of code onward.
fn seated_ground(
    w: &mush_core::World,
    actor: ObjId,
) -> Option<(MatchId, &crate::arena::state::ArenaDefinition)> {
    let registry = w.module_state::<ArenaRegistry>()?;
    let (match_id, _unit) = registry.seat_of_pilot(actor)?;
    Some((match_id, registry.arena_of(match_id)?))
}

/// **`arena.map`'s handler** (16b cut Q.3): the terrain of the arena the asker
/// is flying in, or [`crate::match_::cockpit::map_unseated`].
///
/// `params` is ignored today. A `window` parameter is where doc 16 §11's 10 km
/// path would slice it — `{ "window": [x0, y0, w, h] }` in cells — and the
/// payload's `origin_cell` is in it for exactly that: a renderer written against
/// that key keeps working when the map stops starting at the arena's corner.
struct MapQuery;

impl mush_core::QueryHandler for MapQuery {
    fn query(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let data = match seated_ground(w, actor) {
            // 16b cut V.1: the ground's named spots ride the map, because they
            // are surveyed ground exactly as the terrain is.
            Some((match_id, ground)) => crate::match_::cockpit::map_frame(
                match_id,
                ground.id.as_str(),
                &ground.terrain,
                &ground.key_spots,
            ),
            // Nothing about any match: not its id, not its ground, not whether
            // one is running at all.
            None => crate::match_::cockpit::map_unseated(),
        };
        Ok(mush_proto::OutputEvent::Panel {
            feed: SmolStr::new_static(MAP_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data,
        })
    }
}

/// **`arena.cockpit`'s handler** (16b cut Q.3): the last frame the arena sent
/// this pilot, re-rendered through
/// [`crate::match_::cockpit::status_frame`] — the same mirror `STATUS` and
/// `CONTACTS` read, which carries no notices, because the mirror is stored with
/// them cleared and a notice re-sent is a notice said twice.
///
/// Anybody who is not flying gets
/// [`crate::match_::cockpit::cockpit_cleared`], which is the same payload an
/// ex-pilot's panel was cleared with: a client that resyncs after its match is
/// told the truth by the same two keys twice.
///
/// **Seated is the binding and never the mirror.** The answer is gated on
/// [`ArenaRegistry::seat_of_pilot`] — the same definition [`seated_ground`] and
/// the rest of this module use — and only then read out of the mirror. A mirror
/// row is a cache of the last frame the arena sent, and the pulse that was
/// already staged when a pilot typed `EXIT` is drained afterwards; gating on the
/// row alone would answer an ex-pilot with a whole tank's contact list, locks
/// and ranges on a feed that is a *query* and can be asked at any time from the
/// lobby.
struct CockpitQuery;

impl mush_core::QueryHandler for CockpitQuery {
    fn query(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let data = w
            .module_state::<ArenaRegistry>()
            .filter(|registry| registry.seat_of_pilot(actor).is_some())
            .and_then(|registry| registry.last_frame(actor))
            .map_or_else(crate::match_::cockpit::cockpit_cleared, |held| {
                // 16b cut B.4 (`T-60`): the ranges the pilot was last sent.
                crate::match_::cockpit::status_frame_held(held.match_id, &held.frame, &held.ranges)
            });
        Ok(mush_proto::OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data,
        })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the arena's world-side surface on the **tacsim** module: the frame
/// feed, the egress re-entry handler, the two seat facts, the state slot and
/// `@arena`.
///
/// `egress_capacity` is sized from `[tacsim.arena].ledger.max_unflushed_chunks`:
/// the arena voids a match past that many unacknowledged chunks, so a channel
/// that can hold twice as many per match never becomes the reason one is lost.
///
/// # Errors
///
/// [`ModError`] if the feed name, the verb or the completion handler is already
/// taken.
pub fn register(
    reg: &mut Registrar<'_>,
    egress_capacity: usize,
    arena_cfg: &crate::config::ArenaConfig,
    match_cfg: &crate::config::MatchConfig,
    sensors_cfg: &crate::config::SensorsConfig,
) -> Result<(), ModError> {
    let frames = reg.panel_feed(FRAME_FEED)?;
    // **16b cut Q.3**: the terrain feed beside it, and its handle dropped at
    // once — nothing ever pushes a map. Registering the feed is what advertises
    // `arena.map` in the client manifest and what routes a `PanelSubscribe` to
    // the query handler below. The order of these two registrations is the order
    // `mush-server`'s boot-parity test pins the manifest in.
    drop(reg.panel_feed(MAP_FEED)?);
    reg.query_handler(MAP_FEED, Arc::new(MapQuery))?;
    reg.query_handler(FRAME_FEED, Arc::new(CockpitQuery))?;
    reg.async_completion(Arc::new(ArenaEgress))?;
    // **16b cut V.3 (16c N2)**: the tacsim module's FIRST store-read completion,
    // and its only one — `mush_core::Registrar::store_read_completion` takes one
    // per module. It is the re-entry of both deferred reads the debrief makes:
    // `SIM REPORT`'s one report document and `@arena/report`'s page of the tape
    // (`crate::match_::report`).
    reg.store_read_completion(Arc::new(crate::match_::report::ReportRead))?;
    reg.hook(mush_core::HookPoint::Connected, Arc::new(SeatFacts));
    // 16b cut K.2 (K4): the kernel's new point, and the revival that rides
    // `Connected`'s registrations above on `EventHook::revived`.
    reg.hook(mush_core::HookPoint::Linkdead, Arc::new(SeatFacts));
    reg.hook(mush_core::HookPoint::Disconnected, Arc::new(SeatFacts));
    reg.state(ArenaRegistry::new(ArenaTasks::new(egress_capacity), frames));
    // 16b cut T.3: the match book beside the registry. A second slot rather than
    // three more fields on the first, because the two have different lifetimes —
    // a registry row exists only while a task does, and a `MatchRecord` outlives
    // its task by exactly as long as it takes to write the result.
    reg.state(crate::match_::MatchBook::new());
    // **16b cut T.3, WS03-A1.** The custody transitions write the `unit:<obj>`
    // document, which is `tactics`' namespace, through `mush_tactics::custody`'s
    // governed `Ctx::persist`. The call is made from inside this module's
    // authoritative action, so the crossing is real; it is declared here rather
    // than assumed, and the pinned row is in
    // `crates/mush-contract-tests/module-ns-owners.toml`.
    reg.persist_delegation(
        MODULE,
        mush_tactics::module::NS,
        "16b cut T.3 (doc 16 section 3.1 Custody): the arena's custody transitions. Check-out \
         sets `deployed { match, arena, snapshot }` on each unit's record and check-in clears it, \
         both through mush_tactics::custody -- the tactics crate's own encoder, its own key and \
         its own strict envelope. tacsim never spells a tactics key or a tactics document itself.",
    )?;
    // **16b cut T.3 re-classification.** Cut T.2 registered this verb ReadOnly,
    // because `@arena/status` is a read. This cut adds `start`, `intent` and
    // `abort`: `start` writes a `deployed` marker on every named unit's record
    // and hands the arena custody, and `abort` ends in a check-in that writes
    // every one of them again plus one result document. That is an authoritative
    // lifecycle, so the class moves with the switches, in the same commit as the
    // stanza note.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    let (arena_cfg, match_cfg, sensors_cfg) =
        (arena_cfg.clone(), match_cfg.clone(), sensors_cfg.clone());
    reg.command(
        CommandSpec {
            name: "@arena".into(),
            shape: ArgShape::EqPair,
            switches: crate::match_::register_switches(),
            help: "@arena/status                       (staff) every match this server is \
                   running: its lifecycle state, its tempo, its arena time, how often it has \
                   lagged, how many ledger chunks have been appended and lost, how many cockpit \
                   frames have gone out, and how many ledger writes are still waiting for the \
                   store to acknowledge them; then one line per unit a stranded deployed \
                   marker still holds, with the @arena/release that clears it. A read.\n\
                   @arena/start <arena>=<red #ids>/<blue #ids>   (staff) check those units out \
                   and run a match: ONE authoritative action that marks every unit deployed, \
                   freezes its world placement and hands the arena custody of it.\n\
                   @arena/intent <unit>=startup | heading <deg> | speed <cms>   (staff) one \
                   order, as the unit's own seat.\n\
                   @arena/abort [<match>]              (staff) stop a match; its units check \
                   back in when the arena answers.\n\
                   @arena/reset                        (staff) return the simulator centre to \
                   idle: every slot and pod a finished or failed match left behind is \
                   released, every pilot is out of their pod with the cockpit billet given \
                   back, and every stock tank is put back at its spec and at its authored \
                   muster cell. Idempotent, and says what it found; refused while a match is \
                   still checked out. A player's own `SIM TEAM` does the same thing on a \
                   finished centre, so two pilots need no staff for a rematch.\n\
                   @arena/release <unit>               (staff) clear one unit's stranded \
                   deployed marker: it is restored from its own check-out snapshot and its \
                   cockpit seat is freed. For a unit a refused check-in left unusable; refused \
                   while its match is still live, where @arena/abort is the verb.\n\
                   @arena/pose <callsign>=<x> <y>[ <heading>]   (staff) open that centre slot \
                   (RA, BC: as the board prints it) on that arena cell for the next match, \
                   facing the heading or its side's own; `=off` gives it its authored cell \
                   back, and bare lists what is set. Refused while a match runs; cleared when \
                   the match that used it ends, and by @arena/reset.\n\
                   @arena/report [<match>] [<chunk>]   (staff) read a match's tape itself: \
                   sixteen chunks a request from the chunk named (0 by default), one line an \
                   event with object numbers, contact ids and the draws, positions left out, \
                   under a header with the seed and every unit's pilot, at most 200 lines a \
                   page; its `Next:` goes on from the chunk the page was cut short in. Bare, \
                   the newest match the centre remembers; a running or older match's tape is \
                   read too. A read."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
            if !is_staff(ctx) {
                return Err(CmdError::msg("Permission denied."));
            }
            crate::match_::arena_command(ctx, &arena_cfg, &match_cfg, &sensors_cfg)
        }),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}

/// The egress capacity for a given `[tacsim.arena]` block.
#[must_use]
pub fn egress_capacity(cfg: &crate::config::ArenaConfig) -> usize {
    usize::try_from(cfg.ledger.max_unflushed_chunks)
        .unwrap_or(usize::MAX)
        .saturating_mul(2)
        .max(64)
}

/// The tacsim module's namespace, restated for the host's benefit: the arena's
/// ledger, its registry and its feed are all this module's.
pub const MODULE: &str = NS;

// ---------------------------------------------------------------------------
// Deliberation: the world's half (16b cut N.2)
// ---------------------------------------------------------------------------
//
// Doc 16 §6 splits deliberation across the two authorities, and this is the
// world's side of the split:
//
//   the arena       says WHICH squads have something to decide about, in arena
//                   time, floored by `[tacsim.arena].deliberate_min_ms`
//   the world       ASKS, off its own loop, through the same LLM re-entry seam
//                   worldgen and the NPC turn run on
//   the arena       is handed an ordinary `IntentKind::Order`, or nothing
//   the world       says the one sentence when an order runs out unrenewed
//
// Nothing here waits: `Effect::ModuleAsync` is deferred and the completion arrives
// as an ordinary `WorldCommand::ModuleAsyncDone`, so a provider that takes thirty
// seconds costs the loop one queue entry (doc 16 §6 "Provider failure").

/// **One pulse's deliberation signals** — one request per squad, deferred off the
/// loop.
///
/// The `actor` of the effect is the squad's **commander**, which is a real world
/// object: it is what the completion runs as, what `mush-llm` meters the call
/// against ([`mush_llm::CallCtx::with_actor`]) and therefore what makes per-squad
/// cost a read of the budget ledger rather than a second counter. It never reaches
/// the prompt.
///
/// **16b cut Y.1 — the shield's first half.** A squad under a pilot's live order
/// (`MatchDeliberation::human_holds`) gets no request: the signal is skipped
/// **before** an in-flight slot is taken, so a human's order stands until it
/// runs out or the pilot replaces it, and the machine's commander asks again at
/// the next signal after the expiry.
pub fn on_deliberate(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    signals: &[crate::arena::command::SquadSignal],
) {
    let now_ms = ctx.w.now().saturating_mul(1_000);
    for signal in signals {
        // The roster, for the unit count the brief carries and for the fan-out the
        // reply needs. A signal for a squad the record no longer holds is dropped:
        // the match was aborted under it.
        let Some(units) = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .and_then(|record| {
                record
                    .squads
                    .iter()
                    .find(|roster| roster.id == signal.squad)
                    .map(|roster| roster.units.len())
            })
        else {
            continue;
        };
        {
            let Some(state) = ctx
                .w
                .module_state_mut::<ArenaRegistry>()
                .map(|registry| registry.deliberation.entry(match_id).or_default())
            else {
                return;
            };
            // 16b cut Y.1: a pilot's order stands; nothing is asked over it.
            if state.human_holds(signal.squad, now_ms) {
                continue;
            }
            if state.in_flight >= crate::deliberation::MAX_IN_FLIGHT_PER_MATCH {
                continue;
            }
            state.in_flight = state.in_flight.saturating_add(1);
        }
        let brief = crate::deliberation::SquadBrief::from_signal(
            match_id.0,
            signal,
            u16::try_from(units).unwrap_or(u16::MAX),
        );
        let job = crate::deliberation::DeliberationJob::Deliberate { brief };
        let Ok(payload) = serde_json::to_string(&job) else {
            // A brief is integers and short strings; this cannot fail. If it did,
            // the in-flight count is given back rather than leaked.
            release_in_flight(ctx, match_id);
            continue;
        };
        let correlation = ctx.w.next_correlation_id();
        ctx.out.defer(mush_core::Deferred::Effect(
            mush_core::Effect::ModuleAsync {
                module: SmolStr::new_static(crate::store::NS),
                actor: signal.commander,
                correlation,
                payload,
            },
        ));
    }
}

/// Give one in-flight slot back, whatever the outcome was.
fn release_in_flight(ctx: &mut Ctx<'_>, match_id: MatchId) {
    if let Some(state) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.deliberation.get_mut(&match_id))
    {
        state.in_flight = state.in_flight.saturating_sub(1);
    }
}

/// **One deliberation reply**, back on the world task with a `Ctx`.
///
/// An order is fanned out as one [`IntentKind::Order`] per unit of the squad, each
/// through [`send_order`] — so the arena authorises each of them again, at apply,
/// exactly as it authorises a pilot's. A refusal is doc 16 §6's fallback and costs
/// one counter: the squad is already on doctrine, because doctrine is what a squad
/// with no order runs.
///
/// **16b cut Y.1 — the shield's second half.** An order for a squad a pilot's
/// order still holds (`MatchDeliberation::human_holds`) — a request that was
/// already in flight when the pilot spoke — is dropped after its in-flight slot
/// is given back: the human's order stands until it runs out or the pilot
/// replaces it.
pub fn on_deliberation_reply(ctx: &mut Ctx<'_>, payload: &str) {
    let Ok(reply) = serde_json::from_str::<crate::deliberation::DeliberationReply>(payload) else {
        tracing::warn!("tacsim: undecodable deliberation reply dropped");
        return;
    };
    let match_id = MatchId(reply.match_id());
    release_in_flight(ctx, match_id);
    let now_ms = ctx.w.now().saturating_mul(1_000);
    match reply {
        crate::deliberation::DeliberationReply::Refused { squad, reason, .. } => {
            if let Some(state) = ctx
                .w
                .module_state_mut::<ArenaRegistry>()
                .and_then(|registry| registry.deliberation.get_mut(&match_id))
            {
                state.refused = state.refused.saturating_add(1);
            }
            tracing::debug!(%match_id, %squad, %reason, "arena deliberation produced no order");
        }
        crate::deliberation::DeliberationReply::Ok {
            mut order, model, ..
        } => {
            // 16b cut Y.1: a pilot's live order outranks the machine's.
            if ctx
                .w
                .module_state::<ArenaRegistry>()
                .and_then(|registry| registry.deliberation.get(&match_id))
                .is_some_and(|state| state.human_holds(order.squad, now_ms))
            {
                tracing::debug!(
                    %match_id, squad = %order.squad,
                    "arena deliberation reply dropped under a pilot's order"
                );
                return;
            }
            // The world stamps `issued_ms`: it is a world millisecond by definition,
            // and letting the provider name it would let a model backdate an order.
            order.issued_ms = now_ms;
            let Some(units) = ctx
                .w
                .module_state::<crate::match_::MatchBook>()
                .and_then(|book| book.get(match_id))
                .and_then(|record| {
                    record
                        .squads
                        .iter()
                        .find(|roster| roster.id == order.squad)
                        .map(|roster| roster.units.clone())
                })
            else {
                return;
            };
            let objective = SmolStr::new(order.objective.as_str());
            let mut sent = 0_usize;
            for unit in units {
                if send_order(ctx, match_id, unit, order.clone()).is_ok() {
                    sent = sent.saturating_add(1);
                }
            }
            if sent == 0 {
                return;
            }
            let squad = order.squad;
            let expires_ms = now_ms.saturating_add(i64::from(order.ttl_ms));
            if let Some(state) = ctx
                .w
                .module_state_mut::<ArenaRegistry>()
                .map(|registry| registry.deliberation.entry(match_id).or_default())
            {
                state.issued = state.issued.saturating_add(1);
                state.orders.insert(
                    squad,
                    OrderWatch {
                        expires_ms,
                        objective: objective.clone(),
                        said: false,
                        issuer: None,
                    },
                );
            }
            tracing::debug!(
                %match_id, %squad, %objective, %model, units = sent,
                "arena deliberation issued an order"
            );
        }
    }
}

/// **Send one squad order to the arena that holds `unit`** (16b cut N.2).
///
/// Narrower than [`forward_intent`] and deliberately so: there is no focus, no
/// station and no control lease to check, because there is no seat — the checks
/// [`forward_intent`] makes are all about *a player's authority over a tank*, and a
/// squad order's authority is that the unit is on the reflex controller, which is a
/// fact only the arena holds and which it checks itself
/// ([`crate::arena::intents::Refusal::NotReflex`]).
///
/// The epoch is the one the world last published for the unit's `helm` channel,
/// exactly as [`forward_intent`] stamps one, so an order written before a seat was
/// taken is refused `Epoch` at apply rather than steering a tank a player is now
/// flying.
///
/// # Errors
///
/// One [`IntentRefusal`] word.
pub fn send_order(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    unit: ObjId,
    order: crate::deliberation::Order,
) -> Result<(), IntentRefusal> {
    offer_helm(ctx, match_id, unit, IntentKind::Order(Box::new(order)))
}

/// **Stamp and offer one squad-level `helm` intent to `unit`** — [`send_order`]'s
/// shape, shared since 16b cut Z.2 with [`forward_release`]: the unit must be in
/// `match_id`, the epoch is the one the world last published for the unit's
/// `helm` channel, the seq is the registry's next, and a full or gone task is
/// `Busy` or `Gone`.
fn offer_helm(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    unit: ObjId,
    kind: IntentKind,
) -> Result<(), IntentRefusal> {
    let channel = Channel::Helm;
    let (intent, tasks) = {
        let registry = ctx
            .w
            .module_state_mut::<ArenaRegistry>()
            .ok_or(IntentRefusal::Unavailable)?;
        if registry.match_of_unit(unit) != Some(match_id) {
            return Err(IntentRefusal::NoMatch);
        }
        let intent = Intent {
            unit,
            channel,
            epoch: registry.epoch_of(unit, channel),
            kind,
            seq: registry.next_seq(),
        };
        (intent, registry.tasks.clone())
    };
    match tasks.offer(match_id, ArenaMsg::Intent(intent)) {
        Ok(()) => Ok(()),
        Err(_) if tasks.is_live(match_id) => Err(IntentRefusal::Busy),
        Err(_) => Err(IntentRefusal::Gone),
    }
}

/// **The one in-world notice doc 16 §6 asks for**, said once per squad whose order
/// ran out unrenewed: *"mind link dropped, unit on doctrine"*.
///
/// Who hears it: every **pilot of the squad's own side** in that match, because
/// their AI wingmen just stopped taking orders, and every connected wizard, because
/// staff are who watch a match. The squad's own tanks hear nothing — they have no
/// pilots, which is why they have a commander.
fn say_expired_orders(ctx: &mut Ctx<'_>) {
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let mut due: Vec<(MatchId, crate::deliberation::SquadId, SmolStr)> = Vec::new();
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (match_id, state) in &mut registry.deliberation {
            for (squad, watch) in &mut state.orders {
                if !watch.said && now_ms >= watch.expires_ms {
                    watch.said = true;
                    due.push((*match_id, *squad, watch.objective.clone()));
                }
            }
        }
    }
    for (match_id, squad, objective) in due {
        let line = format!(
            "Squad {squad}: {} (its {objective} order has run out).",
            crate::deliberation::MIND_LINK_DROPPED
        );
        let pilots: Vec<ObjId> = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .map(|record| {
                record
                    .teams
                    .iter()
                    .filter(|roster| roster.team == squad.team)
                    .flat_map(|roster| roster.pilots.iter().map(|(pilot, _)| *pilot))
                    .collect()
            })
            .unwrap_or_default();
        for pilot in pilots {
            if ctx.w.get(pilot).is_some() {
                ctx.out.sys_to_actor(pilot, NOTICE_CATEGORY, line.clone());
            }
        }
        notify_staff(ctx, format!("Arena match {match_id}: {line}"));
    }
}
