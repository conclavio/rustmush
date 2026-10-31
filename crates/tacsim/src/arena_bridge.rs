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
//! **16b cut I.4** (16c N10) put the staff watchers on it: who watches which
//! match (at most [`crate::arena_bridge::MAX_WATCHERS`] a match), the feed their picture goes out on
//! ([`crate::arena_bridge::OBSERVER_FEED`]) and the last picture each watched match sent. A watcher
//! is never a pilot — [`crate::arena_bridge::watch`] refuses anybody holding a centre slot or a seat —
//! and the picture reaches watchers and nobody else.
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

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mush_core::module::{AsyncCompletion, ModError, Registrar};
use mush_core::{
    ArgShape, Audience, ChannelId, CmdError, CommandSpec, Ctx, Ledger, ObjId, PanelFeedHandle,
};
use smol_str::SmolStr;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::arena::card::StockCard;
use crate::arena::clock::Tempo;
use crate::arena::intents::{Control, ControlState, Intent, IntentKind};
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena::observer::Observer;
use crate::arena::state::{Channel, ContactId, Controller, MatchEnd, Power, StopReason};
use crate::arena::step::Frame;
use crate::arena_task::{ArenaHandle, ArenaMsg, ArenaOut, MatchId};
use crate::deliberation::CharacterRef;
// 17b cut M3.5: `n` with a comma every three digits, how a player line writes
// an op budget — the one helper the pool's sentence shares.
use crate::match_::pool::thousands;
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

/// **The panel feed a replay goes out on** (16b cut I.1; 16c N9, Q37).
///
/// Two things ride it. `SIM REPLAY` **pushes** one keyframe's window to the
/// actor who typed it — a `grid` the line client prints as it prints `VIEW`'s
/// (`crate::match_::replay::replay_window`), carrying `schema.retain =
/// "latest"` so stepping never pushes the scene out of a reconnect's window. And
/// the query handler [`register`] installs **answers** the whole replay
/// document the actor's replay cursor names
/// (`crate::match_::replay::replay_answer`), under a `replay` shape a line
/// client prints nothing for. Registered as a feed so it is in the client
/// manifest and a `PanelSubscribe` routes to that handler; after `arena.map`
/// in the manifest order `mush-server`'s boot-parity test pins.
pub const REPLAY_FEED: &str = "arena.replay";

/// **The panel feed the staff picture goes out on** (16b cut I.4; 16c N10).
///
/// **Pushed** to a match's watchers — never to a pilot, never to anybody else —
/// by `on_frames` whenever the arena's `Frames` carries a picture (at most
/// once an arena second), as [`crate::match_::observer::observer_frame`]'s
/// keyframe, and answered by the query handler [`register`] installs to a
/// watcher who resyncs. Anybody who is not watching is answered
/// [`crate::match_::observer::observer_cleared`], and a watch that ends is sent
/// the same. The payload declares no `shape`, so a line client prints nothing
/// for it. After `arena.replay` in the manifest order `mush-server`'s
/// boot-parity test pins.
pub const OBSERVER_FEED: &str = "arena.observer";

/// **At most this many staffers watch one match** (16b cut I.4; 16b cut U.2's
/// bound): at most one picture per watcher per pulse, so at most eight
/// `arena.observer` events on any pulse, far under
/// `[tacsim.harness].max_output_events_per_pulse`.
pub const MAX_WATCHERS: usize = 8;

/// `@arena/watch`'s refusal for anybody who flies, or is about to (16b cut
/// I.4): a centre slot of the actor's own or of any character of their
/// account, or a seat binding. A dropped or `EXIT`ed pilot keeps their slot.
pub const WATCH_FLYING: &str =
    "You are flying, or about to; EXIT and leave your slot before you watch.";

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
    bulk_acks: Vec<(u64, crate::minds::bulk::commit::CommitLatch)>,
    /// The egress capacity the table was built with (16b cut AF.3,
    /// [`ArenaTasks::capacity`]).
    capacity: usize,
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
            bulk_acks: Vec::new(),
            capacity: capacity.max(1),
        })))
    }

    /// **The egress capacity this table was built with** (16b cut AF.3): what
    /// [`ArenaRegistry::fresh`] builds a new process's table at.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.0.lock().ok().map_or(1, |tasks| tasks.capacity)
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
    pub(crate) fn await_bulk_write(&self, latch: crate::minds::bulk::commit::CommitLatch) {
        if let Ok(mut tasks) = self.0.lock() {
            if tasks.bulk_acks.len() < 8 {
                tasks.bulk_acks.push((0, latch));
            } else {
                tracing::error!("bulk durable-writer latch bound exceeded; work remains stopped");
            }
        }
    }

    pub fn stamp_ledger_writes(&self, serial: u64) {
        if let Ok(mut tasks) = self.0.lock() {
            for (on, _) in &mut tasks.bulk_acks {
                if *on == 0 {
                    *on = serial;
                }
            }
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
            let mut bulk_ready = false;
            tasks.bulk_acks.retain(|(serial, latch)| {
                if *serial == 0 || *serial > high_water {
                    return true;
                }
                latch.acknowledge();
                bulk_ready = true;
                false
            });
            if bulk_ready {
                let _ = tasks.egress_tx.try_send(ArenaOut::MindBulkReady);
            }
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

struct ArenaEgressReceiver {
    rx: mpsc::Receiver<ArenaOut>,
    tasks: ArenaTasks,
}

impl mush_core::OffLoopEgressReceiver for ArenaEgressReceiver {
    fn recv_and_stage(
        &mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<usize>> + Send + '_>> {
        Box::pin(async move {
            let first = self.rx.recv().await?;
            let mut staged = 0_usize;
            if self.tasks.stage(first) {
                staged += 1;
            }
            while staged < MAX_EGRESS_PER_DRAIN {
                match self.rx.try_recv() {
                    Ok(message) => {
                        if !self.tasks.stage(message) {
                            break;
                        }
                        staged += 1;
                    }
                    Err(_) => break,
                }
            }
            Some(staged)
        })
    }
}

impl mush_core::OffLoopSimulationHost for ArenaTasks {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn module_name(&self) -> &'static str {
        MODULE
    }

    fn egress_payload(&self) -> &'static str {
        EGRESS_PAYLOAD
    }

    fn take_egress(&self) -> Option<Box<dyn mush_core::OffLoopEgressReceiver>> {
        let rx = ArenaTasks::take_egress(self)?;
        Some(Box::new(ArenaEgressReceiver {
            rx,
            tasks: self.clone(),
        }))
    }

    fn stamp_ledger_writes(&self, serial: u64) {
        ArenaTasks::stamp_ledger_writes(self, serial);
    }

    fn ack_ledger_upto(&self, high_water: u64) {
        ArenaTasks::ack_ledger_upto(self, high_water);
    }

    fn signal_stop_all(&self) {
        ArenaTasks::signal_stop_all(self);
    }

    fn shutdown(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            ArenaTasks::shutdown(self, JOIN_TIMEOUT).await;
        })
    }
}

// ---------------------------------------------------------------------------
// The module state slot
// ---------------------------------------------------------------------------

/// What the world remembers about one match, for `@arena/status` and for cut
/// T.3's check-in.
#[derive(Clone, Debug, Default)]
pub struct MatchMirror {
    /// Bounded RAM-only own-hull diagnostics, empty after restart.
    pub traces: crate::arena::author_trace::TraceMirror,
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
    /// **The staff tempo floor this match's task was sent** (16b cut I.3; 16c
    /// N10), milliseconds, or `None`. Written by `@arena/tempo`
    /// ([`ArenaRegistry::note_tempo_floor`]) **only after the task took the
    /// message**, so `@arena/status` never claims a floor the arena does not
    /// have; gone with the mirror when the match is untracked. What the arena
    /// itself holds is `Clock::floor_ms`; this is the world's copy of the last
    /// set it delivered.
    pub tempo_floor_ms: Option<u32>,
    /// **The arena millisecond a staff tempo floor was last lifted at** (16b cut
    /// AP.1, 16c `B-86`), or `None` before the first lift and after every
    /// floor.
    ///
    /// Written by [`ArenaRegistry::note_tempo_floor`] alone: a lift (`None`)
    /// stamps it with [`MatchMirror::now_ms`] as the mirror holds it at that
    /// moment — whether or not a floor stood, since `@arena/tempo <match>=off`
    /// is a lift either way, and a stamp with no floor behind it only leaves the
    /// frames already held unaged until their next — and a new floor (`Some`)
    /// clears it. It exists for the cockpit's aged mirror
    /// (`crate::match_::cockpit::aged_weapons`): under a floor the wall clock
    /// runs ahead of the arena's, and a frame the pilot was sent under one
    /// carries that same wall time with it after the lift — so a frame whose
    /// [`CockpitFrame::arena_ms`] is at or before this stamp keeps its own
    /// countdown, unaged, until the next frame the arena sends that pilot. A
    /// frame goes out on a change and never for a countdown (the gun's own
    /// ready transition is one such change), and it carries an arena time past
    /// the lift.
    pub floor_lifted_at_ms: Option<i64>,
    /// **The arena millisecond of the last snapshot the world wrote** (16b cut
    /// AF.2), or `None` before the first: `on_snapshot` sets it as it writes
    /// `tacsim:snapshot:<n>`, the live record carries it as its `at_ms`, and
    /// `@arena/status` prints it.
    pub snapshot_at_ms: Option<i64>,
    /// **That snapshot's state hash** (16b cut AF.2), beside it.
    pub snapshot_hash: Option<u64>,
    /// **The snapshot moment this match was last resumed from** (16b cut AF.2
    /// carries it on the live record; 16b cut AF.3 sets it), or `None`.
    pub resumed_from_ms: Option<i64>,
    /// **The match's captured Compact floor**, per mille (17b cut M1b.5; doc 17
    /// §5, D-UM-17 as amended): the floor the arena itself holds
    /// (`Arena::odds_floor_permille`), written at [`open_match`] from the SAME
    /// `lobby::minds_config` read `spawn_arena` captured, and at boot resume
    /// ([`resume_match`]) from the decoded arena's own field — **never** the
    /// live config, which a reload may have moved under a running match.
    /// `@mind/live`'s `min_odds` pre-check reads it; `0` on a mirror nothing
    /// opened (no match, no pre-check).
    pub odds_floor_permille: u32,
    /// **Who flies each unit, as far as the world knows** (17b cut M1b.5):
    /// written at check-out from the custody units' controllers (and at resume
    /// from the decoded arena's) and on every [`Control`] the world delivers —
    /// `Live` is the pilot's, `Released` the mind's (every hull since 17b cut
    /// M4.3; before it the reflex controller's, or since 17b cut M3.5 the
    /// mind's for a hull whose arena carried a mind row) — exactly as
    /// `apply_control` moves the arena's own row. `@mind/live`
    /// refuses a hull that is not [`Controller::Mind`] here first; the arena refuses it again
    /// at apply (`not_mind`). Kept here because no cheaper world-side read
    /// exists: the staff picture knows the controllers only while somebody
    /// watches.
    pub controllers: BTreeMap<ObjId, Controller>,
    /// **The hulls checked out with a weapon** (17b cut M1b.5): the read-back's
    /// fire-discipline section is shown live for these and inert (`no weapons
    /// station on this hull`, doc 17 §11) for every other. Written at check-out
    /// and at resume; a hull that loses its last gun mid-match is still listed
    /// (the world hears no `RemoveWeapons`; the arena's own row is the truth).
    pub armed: BTreeSet<ObjId>,
    /// **The mind cards as the world last learned them** (17b cut M1b.5; doc
    /// 17 §11, §22's M1b): per hull, the card, the arena millisecond and where
    /// the knowledge came from — [`CardSource::OnTape`] when `on_chunk` read a
    /// `mind_card` line (the arena's own row, as applied), [`CardSource::Sent`]
    /// when `@mind/live` offered a change and no line for it has arrived since
    /// (the verb's scratch card at the mirror's arena clock), and
    /// [`CardSource::CheckOut`] for every mind hull at [`open_match`] (its
    /// first sealed profile's card on the match's doctrine — Line for a hull
    /// nobody chose a mind for — at arena millisecond nought; 17b cut M5.2).
    /// A seat's `MIND PROFILE` is a `mind_card` line too (17b cut M5.2), so
    /// the mirror follows a switch: the card that will fly. Absent after a
    /// resume until a line arrives — the tape's last `mind_card` line names the
    /// card, and the read-back says so. The tape is read per closed chunk, so
    /// a line reaches the mirror when its chunk closes (the read-back's lag).
    pub cards: BTreeMap<ObjId, (StockCard, i64, CardSource)>,
}

/// **Where the world's copy of a mind's card came from** (17b cut M1b.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardSource {
    /// The check-out's: every mind row opens on its hull's first sealed
    /// profile over the match's own doctrine (`MindState::fresh_with`; `line`
    /// for a hull nobody chose a mind for, 17b cut M5.2), which the world
    /// knows without the tape.
    CheckOut,
    /// A `mind_card` line on the tape: the card the arena applied.
    OnTape,
    /// An offer `@mind/live` made, not yet on the tape.
    Sent,
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
    /// **The arena's own clock after the step that built this frame**,
    /// milliseconds (16b cut AP.1, 16c `B-86`): the
    /// [`crate::arena_task::ArenaOut::Frames`] message's `now_ms`, written by
    /// `on_frames`. A fact about **what** the pilot was told — the cockpit reads
    /// it against [`MatchMirror::floor_lifted_at_ms`] to tell a frame built under
    /// a staff tempo floor from one built after its lift.
    pub arena_ms: i64,
    /// **The world's instant `on_frames` stored this frame** (16b cut AP.1, 16c
    /// `B-86`): a fact about **when** the pilot was told it. The cockpit's
    /// `FIRE` pre-check and `WEAPONSTATUS` age each weapon's countdown by the
    /// wall time since
    /// ([`crate::match_::cockpit::aged_weapons`]), because a frame is never
    /// re-sent for a countdown change and a pilot must not be refused locally
    /// for a gun that is already ready in the arena.
    ///
    /// A `std::time::Instant` and not an arena or world number: the world has
    /// no millisecond clock (`World::now()` is unix seconds), and this file is
    /// the world side of the seam — nothing under `crate::arena` reads a wall
    /// clock, and neither field here crosses the seam or folds into any hash.
    pub received: Instant,
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
    /// **The switch each pilot last forwarded to their pod** (16b cut O.2, 16c
    /// N12; the cut O plan's item 38), pilot to `(on, until_s)`: the `EMCON`
    /// the world has just sent, until the world second `until_s`.
    ///
    /// The `Starting` hint's reason again: an intent is buffered to the next
    /// pulse and never steps the arena, so the mirror's `ecm` is up to one
    /// `Quiet` period stale, and without this `EMCON off` then `EMCON on` inside
    /// one pulse would answer `EMCON is already on.` to the second — true of the
    /// mirror, false of the tank the pilot has just switched. The cockpit reads
    /// this **before** the mirror ([`ArenaRegistry::emcon_hint`]), so the two
    /// orders are both forwarded with true replies and the arena, resolving
    /// them in order, ends where the pilot left it.
    ///
    /// Stamped at forward with the expiry `STARTUP` computes (one second past a
    /// whole `Quiet` period), and it ends the four ways
    /// [`ArenaRegistry::starting_hints`] does, read for the switch:
    /// 1. **confirmed** — [`on_frames`] removes it when a frame for that pilot
    ///    carries `ecm == Some(on)`, the arena agreeing with the pilot;
    /// 2. **refused** — [`on_frames`] removes it when a frame for that pilot
    ///    carries a [`crate::arena::perception::FrameNotice::Refused`] whose
    ///    `order` is `emcon`;
    /// 3. **expired** — the world clock passes the stamped second, and the
    ///    mirror answers again;
    /// 4. **unbound** — [`ArenaRegistry::unbind_pilot`] and
    ///    [`ArenaRegistry::untrack`] drop it wherever they drop the `Starting`
    ///    hint.
    ///
    /// A frame whose `ecm` disagrees and that carries no such refusal does
    /// *not* clear it: a batch staged before the intent arrived is drained
    /// after it.
    emcon_hints: BTreeMap<ObjId, (bool, i64)>,
    /// **Who wants the line-mode picture** (16b cut AP.3, 16c N19): the pilots
    /// who typed `WATCH ON`, each pushed
    /// [`crate::match_::cockpit::watch_lines`]' block by [`on_frames`] after a
    /// keyframe whose block reads differently from the last one pushed.
    ///
    /// A view and never authority: nothing here crosses the seam, folds into a
    /// hash or changes what the arena does. **Not the staff watchers**
    /// ([`ArenaRegistry::watchers`], 16b cut I.4), which are staffers fed the
    /// whole match's picture on the observer feed; this is a seated pilot asking
    /// for their own picture as text. Keyed on the pilot the registry binds,
    /// set and cleared only by [`ArenaRegistry::set_line_watch`], and dropped
    /// with the seat ([`ArenaRegistry::unbind_pilot`]) and with the match
    /// ([`ArenaRegistry::untrack`]): the switch is the seat's, and a pilot who
    /// boards again types it again.
    line_watch: BTreeSet<ObjId>,
    /// **The block last pushed to each watching pilot** (16b cut AP.3), the
    /// text [`ArenaRegistry::note_watch_block`] compares the next one against.
    ///
    /// `Frame::same_as` re-sends a frame for a moved cell, a moved pose, the
    /// arena's tempo and a held contact's age at every whole second, none of
    /// which the block prints; without this comparison a pilot driving
    /// straight would be sent the same lines twice a second, and the rate they
    /// arrived at would read out the tempo 16c `T-59` took off `STATUS`. Dropped
    /// wherever [`ArenaRegistry::line_watch`] is, and by `WATCH OFF`.
    last_watch_block: BTreeMap<ObjId, String>,
    /// **16b cut K.2**: the ground each running match is fought over, so `VIEW`
    /// can draw the window without re-deriving a 65,536-cell arena from the
    /// atlas on every look. One `Arc` per match, taken at check-out and dropped
    /// with the match.
    arenas: BTreeMap<MatchId, Arc<crate::arena::state::ArenaDefinition>>,
    /// What each running match's squads have been told and when each order
    /// runs out: ordinary offers use world time; resumed accepted orders use
    /// the match's actually published arena time.
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
    /// **Who watches which match** (16b cut I.4; 16c N10), staffer to match.
    /// Written only by [`ArenaRegistry::bind_watcher`] and
    /// [`ArenaRegistry::unbind_watcher`]; at most [`MAX_WATCHERS`] rows a match
    /// ([`watch`] refuses the ninth). A watcher holds no slot and no seat.
    watchers: BTreeMap<ObjId, MatchId>,
    /// **The panel feed the staff picture goes out on** (16b cut I.4):
    /// [`OBSERVER_FEED`]'s handle, kept because it is pushed on.
    observers: PanelFeedHandle,
    /// **The last staff picture each watched match sent** (16b cut I.4), what
    /// the `arena.observer` query re-renders for a watcher who resyncs. Dropped
    /// when a match's last watcher goes and by [`ArenaRegistry::untrack`].
    last_observer: BTreeMap<MatchId, Observer>,
}

/// **One match's deliberation state, on the world side** (16b cut N.2).
#[derive(Clone, Debug, Default)]
pub struct MatchDeliberation {
    /// Requests deferred and not yet completed. Bounded by
    /// [`crate::deliberation::MAX_IN_FLIGHT_PER_MATCH`] so a match whose provider
    /// has stopped answering cannot queue one per pulse for ever.
    in_flight: usize,
    /// Per squad: its explicit deadline clock and what the order said.
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
    /// **Read through 17b cut L.1** (doc 17 §4, D-UM-53): the shield is a live
    /// order by a character **other than the squad's own lance-leader**. Today that
    /// is exactly a seated pilot's order — its watch has an issuer, the pilot's own
    /// tank — while a leader's order is written in the commander's scope
    /// (`issuer: None`), so a persona's order never shields its own deliberation.
    /// Recovered watches compare the actual published arena clock; ordinary
    /// watches retain the original world-clock policy.
    ///
    /// Read by both halves of the shield — [`on_deliberate`] spends no signal on
    /// such a squad and [`on_deliberation_reply`] drops an order for it — so a
    /// human's order stands until it runs out or the pilot replaces it, and the
    /// machine's commander asks again at the next signal after the expiry.
    #[must_use]
    pub(crate) fn human_holds(
        &self,
        squad: crate::deliberation::SquadId,
        now_ms: i64,
        arena_ms: Option<i64>,
    ) -> bool {
        self.orders
            .get(&squad)
            .is_some_and(|watch| watch.issuer.is_some() && !watch.deadline.due(now_ms, arena_ms))
    }
}

/// One squad's order, as the world remembers it.
#[derive(Clone, Debug)]
struct OrderWatch {
    /// Existing offers use world time; recovered native orders use arena time.
    deadline: OrderDeadline,
    /// The objective, by its word, for the sentence.
    objective: SmolStr,
    /// Whether the expiry has already been said. A notice is said once.
    said: bool,
    /// **Whose scope it was written in** (16b cut Y.1): the pilot's own tank for
    /// an `ORDER`, `None` for the squad's lance-leader (the commander's scope).
    /// What `MatchDeliberation::human_holds` reads, and what cut Y.2's span counts
    /// per issuer.
    issuer: Option<ObjId>,
    /// **Who gave it** (17b cut L.1; D-UM-52): the order's author, the seat's
    /// character for an `ORDER` and the squad's lance-leader for a deliberated one.
    /// Read by [`ArenaRegistry::order_author`], and named by the lapse sentence
    /// (17b cut L.2, `say_expired_orders`), which it also addresses.
    by: CharacterRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OrderDeadline {
    World(i64),
    Arena(i64),
}
impl OrderDeadline {
    fn due(self, world_ms: i64, arena_ms: Option<i64>) -> bool {
        match self {
            Self::World(end) => world_ms >= end,
            Self::Arena(end) => arena_ms.is_some_and(|now| now >= end),
        }
    }
}

impl ArenaRegistry {
    fn restore_order_watches(
        &mut self,
        match_id: MatchId,
        recovered_orders: Vec<(
            crate::deliberation::SquadId,
            CharacterRef,
            Option<ObjId>,
            &'static str,
            i64,
        )>,
    ) {
        // Restore actual accepted attribution only: no command, counters, or requests.
        let state = self.deliberation.entry(match_id).or_default();
        for (squad, by, issuer, objective, expiry) in recovered_orders {
            state.orders.insert(
                squad,
                OrderWatch {
                    deadline: OrderDeadline::Arena(expiry),
                    objective: SmolStr::new_static(objective),
                    said: false,
                    issuer,
                    by,
                },
            );
        }
    }

    /// A registry over `tasks`, publishing frames on `frames` and (16b cut
    /// I.4) the staff picture on `observers`.
    #[must_use]
    pub fn new(
        tasks: ArenaTasks,
        frames: PanelFeedHandle,
        observers: PanelFeedHandle,
    ) -> ArenaRegistry {
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
            emcon_hints: BTreeMap::new(),
            line_watch: BTreeSet::new(),
            last_watch_block: BTreeMap::new(),
            arenas: BTreeMap::new(),
            deliberation: BTreeMap::new(),
            order_stamps: BTreeMap::new(),
            watchers: BTreeMap::new(),
            observers,
            last_observer: BTreeMap::new(),
        }
    }

    /// **The registry `register` would build at a new process's boot**, over
    /// this one's two feed handles, with an [`ArenaTasks`] of the same capacity
    /// (16b cut AF.3): no task, no binding, no mirror row and a fresh
    /// [`Ledger`]. What an in-process test uses to stand for a server-half
    /// restart — the old table's tasks stopped and joined first, as the
    /// shutdown tail does; production never calls it, because a new process
    /// registers its own.
    #[must_use]
    pub fn fresh(old: &ArenaRegistry) -> ArenaRegistry {
        ArenaRegistry::new(
            ArenaTasks::new(old.tasks.capacity()),
            old.frames.clone(),
            old.observers.clone(),
        )
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
    /// per squad it reached, with `issuer` the pilot's own tank and (17b cut L.1)
    /// `by` the seat's character.
    pub fn note_human_order(
        &mut self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
        objective: SmolStr,
        issuer: ObjId,
        by: CharacterRef,
        expires_ms: i64,
    ) {
        let state = self.deliberation.entry(match_id).or_default();
        state.orders.insert(
            squad,
            OrderWatch {
                deadline: OrderDeadline::World(expires_ms),
                objective,
                said: false,
                issuer: Some(issuer),
                by,
            },
        );
        state.human_issued = state.human_issued.saturating_add(1);
    }

    /// **Whose scope the order the world remembers on `squad` was written in**
    /// (16b cut Y.1): the pilot's own tank for an `ORDER`, and `None` for the
    /// squad's lance-leader (the commander's scope) or a squad with no watch at
    /// all. Since 17b cut L.1 (D-UM-52) this is never who gave the order — that
    /// is a character, never a unit, and [`ArenaRegistry::order_author`] reads it.
    /// Expired or not: it is the watch's record, and
    /// `MatchDeliberation::human_holds` is what asks whether it still stands. A
    /// read for tests and for cut Y.2's span.
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

    /// **Who authored the order the world remembers on `squad`** (17b cut L.1;
    /// D-UM-52): the watch's `by` — the seat's character for an `ORDER`, the
    /// squad's lance-leader for a deliberated one — or `None` for a squad with no
    /// watch. Expired or not, as [`ArenaRegistry::order_issuer`]. A read for the
    /// tests; the lapse sentence (17b cut L.2, `say_expired_orders`) reads the same
    /// field off the watch it is saying.
    #[must_use]
    pub fn order_author(
        &self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
    ) -> Option<CharacterRef> {
        self.deliberation
            .get(&match_id)
            .and_then(|state| state.orders.get(&squad))
            .map(|watch| watch.by.clone())
    }

    /// **The squads `issuer` holds a live order on** (16b cut Y.2; doc 16 §2
    /// W8's span of control): every squad of `match_id` whose world watch names
    /// `issuer` and has not run out on its captured clock. `now_ms` applies to
    /// ordinary watches; recovered watches read this match's published clock.
    ///
    /// An ordinary watch keeps its original world deadline and runs its full time to
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
                    .filter(|(_, watch)| {
                        watch.issuer == Some(issuer)
                            && !watch
                                .deadline
                                .due(now_ms, self.matches.get(&match_id).map(|m| m.now_ms))
                    })
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

    /// A fully offered release by the side's pilot lead ends the squad's
    /// commander watch too. A wingman's release never calls this: it affects
    /// only one machine. Partial offers retain the watch for the others.
    pub(crate) fn note_commander_order_released(
        &mut self,
        match_id: MatchId,
        squad: crate::deliberation::SquadId,
    ) {
        if let Some(state) = self.deliberation.get_mut(&match_id)
            && state
                .orders
                .get(&squad)
                .is_some_and(|watch| watch.issuer.is_none())
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
            // 16b cut O.2: and the pod's switch hint, for the same reason.
            self.emcon_hints.remove(&pilot);
            // **16b cut AP.3**: and the line-mode switch with its last block.
            // A cleared cockpit never leaks a digest, and the next match's
            // pilot starts with the switch off.
            self.line_watch.remove(&pilot);
            self.last_watch_block.remove(&pilot);
        }
        self.arenas.remove(&match_id);
        // 16b cut N.2: and the deliberation row, for the same reason. An order
        // whose time to live outlives the match it was written for would otherwise
        // say its lapse sentence ("… order has run out; back on their minds", 17b
        // cut L.2, worded by 17b cut M5.6) about a squad that no longer exists, and an
        // in-flight count left behind would be a permanent debit against the *next*
        // match that took this id.
        self.deliberation.remove(&match_id);
        // **16b cut Y.2**: and the order rate's window for every tank of the
        // match. The issuer is the pilot's own tank, and a tank's next match
        // starts with a commander who has given nothing yet.
        for unit in &units {
            self.order_stamps.remove(unit);
        }
        // **16b cut I.4**: and the last staff picture. The watchers themselves
        // are [`crate::match_::finish`]'s to unbind, BEFORE this call, because
        // each is owed a cleared picture and this has no `Ctx` to send one.
        self.last_observer.remove(&match_id);
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

    /// **Record the staff tempo floor a match's task accepted** (16b cut I.3):
    /// [`MatchMirror::tempo_floor_ms`], written by `@arena/tempo` after a
    /// successful offer and never before. A match this registry does not track
    /// is left alone.
    ///
    /// **16b cut AP.1** (16c `B-86`): and [`MatchMirror::floor_lifted_at_ms`]
    /// beside it — a lift stamps the mirror's own [`MatchMirror::now_ms`], the
    /// last arena time the world has heard, and a floor clears the stamp, so the
    /// cockpit's aged mirror leaves a frame built under the floor unaged.
    pub fn note_tempo_floor(&mut self, match_id: MatchId, floor_ms: Option<u32>) {
        if let Some(mirror) = self.matches.get_mut(&match_id) {
            mirror.tempo_floor_ms = floor_ms;
            mirror.floor_lifted_at_ms = match floor_ms {
                Some(_) => None,
                None => Some(mirror.now_ms),
            };
        }
    }

    /// **Record what the world knows of one mind's card** (17b cut M1b.5):
    /// [`MatchMirror::cards`], written by `@mind/live` at an offer (`Sent`) and
    /// by `on_chunk` from the tape (`OnTape`). A match this registry does not
    /// track is left alone.
    pub fn note_card(
        &mut self,
        match_id: MatchId,
        unit: ObjId,
        card: StockCard,
        t_ms: i64,
        source: CardSource,
    ) {
        if let Some(mirror) = self.matches.get_mut(&match_id) {
            mirror.cards.insert(unit, (card, t_ms, source));
        }
    }

    /// Every tracked match, in id order.
    pub fn tracked(&self) -> impl Iterator<Item = (MatchId, &MatchMirror)> {
        self.matches.iter().map(|(id, mirror)| (*id, mirror))
    }

    /// **A snapshot reached the world** at `at_ms` (doc 32 cut CF.2: the field host's
    /// snapshot path, which writes no match record).
    pub fn note_snapshot(&mut self, match_id: MatchId, at_ms: i64) {
        if let Some(mirror) = self.matches.get_mut(&match_id) {
            mirror.snapshot_at_ms = Some(at_ms);
        }
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

    /// **Bind a staffer as a watcher of that match** (16b cut I.4) — the
    /// registry's own bookkeeping and nothing else: no check, no message to
    /// the task, nothing sent. [`watch`] calls it after its checks; a test rig
    /// whose match has no lifecycle record around it calls it directly. A
    /// staffer already watching another match is moved (one row a watcher).
    pub fn bind_watcher(&mut self, actor: ObjId, match_id: MatchId) {
        self.watchers.insert(actor, match_id);
    }

    /// **Forget one watcher** (16b cut I.4), answering the match they watched —
    /// the undo of [`Self::bind_watcher`], bookkeeping only. When that was the
    /// match's last watcher its last picture goes with it, so a later watcher
    /// is never answered a stale one. [`unwatch`] is what also clears the
    /// panel and tells the task.
    pub fn unbind_watcher(&mut self, actor: ObjId) -> Option<MatchId> {
        let match_id = self.watchers.remove(&actor)?;
        if !self.watchers.values().any(|held| *held == match_id) {
            self.last_observer.remove(&match_id);
        }
        Some(match_id)
    }

    /// **Everybody watching that match** (16b cut I.4), in object order — at
    /// most [`MAX_WATCHERS`].
    #[must_use]
    pub fn watchers_of(&self, match_id: MatchId) -> Vec<ObjId> {
        self.watchers
            .iter()
            .filter(|(_, held)| **held == match_id)
            .map(|(watcher, _)| *watcher)
            .collect()
    }

    /// **The match that staffer watches** (16b cut I.4), if any.
    #[must_use]
    pub fn watching(&self, actor: ObjId) -> Option<MatchId> {
        self.watchers.get(&actor).copied()
    }

    /// **The last staff picture that match sent** (16b cut I.4), while it is
    /// watched.
    #[must_use]
    pub fn last_observer(&self, match_id: MatchId) -> Option<&Observer> {
        self.last_observer.get(&match_id)
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
        // 16b cut O.2: and the pod's switch hint, for the same reason.
        self.emcon_hints.remove(&pilot);
        // **16b cut AP.3**: and the line-mode switch, which is the seat's — a
        // pilot who boards again (a reclaim included) types `WATCH ON` again.
        self.line_watch.remove(&pilot);
        self.last_watch_block.remove(&pilot);
        self.pilots.remove(&pilot).is_some()
    }

    /// **Set or clear one pilot's line-mode switch** (16b cut AP.3, 16c N19):
    /// `WATCH ON` (`on`) or `WATCH OFF`. Off also forgets the last block pushed,
    /// so a pilot who turns it on again is sent the block afresh. The staff
    /// watchers (`@arena/watch`, [`ArenaRegistry::bind_watcher`]) are another
    /// thing entirely.
    pub fn set_line_watch(&mut self, pilot: ObjId, on: bool) {
        if on {
            self.line_watch.insert(pilot);
        } else {
            self.line_watch.remove(&pilot);
            self.last_watch_block.remove(&pilot);
        }
    }

    /// **Whether that pilot has typed `WATCH ON`** on the seat they hold (16b
    /// cut AP.3). Read once per frame by `on_frames` — and the digest is built
    /// only when it answers `true`, so a pilot who is not watching costs nothing.
    #[must_use]
    pub fn line_watching(&self, pilot: ObjId) -> bool {
        self.line_watch.contains(&pilot)
    }

    /// **Record the block about to be pushed to that pilot, if it is new** (16b
    /// cut AP.3): `true`, and `block` recorded, when it differs from the last
    /// block pushed to them or there is none; `false`, and nothing written,
    /// when it is the same text. The bridge pushes only on `true`; the `WATCH
    /// ON` verb pushes either way (the pilot asked) and records through here so
    /// the next frame's identical block is not repeated.
    pub fn note_watch_block(&mut self, pilot: ObjId, block: &str) -> bool {
        if self
            .last_watch_block
            .get(&pilot)
            .is_some_and(|last| last == block)
        {
            return false;
        }
        self.last_watch_block.insert(pilot, block.to_owned());
        true
    }

    /// **Stamp the `Starting` hint for one pilot** (16b cut D.1), expiring at the
    /// world second `until_s`. See `ArenaRegistry::starting_hints` for the four
    /// ways it ends.
    pub fn hint_starting(&mut self, pilot: ObjId, until_s: i64) {
        self.starting_hints.insert(pilot, until_s);
    }

    /// **Move that pilot's held frame's receipt back by `by`** (16b cut AP.1, 16c
    /// `B-86`), so a test can reach a weapon's ready instant on the aged mirror
    /// ([`CockpitFrame::received`]) without sleeping. A pilot with no frame is
    /// left alone, and a receipt that cannot go back that far stays where it is.
    /// Production never calls it.
    pub fn age_last_frame(&mut self, pilot: ObjId, by: Duration) {
        if let Some(held) = self.last_frames.get_mut(&pilot) {
            held.received = held.received.checked_sub(by).unwrap_or(held.received);
        }
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

    /// **Stamp the pod's switch hint for one pilot** (16b cut O.2): the pilot
    /// has just forwarded `EMCON on` (`on`) or `EMCON off`, and the hint stands
    /// until the world second `until_s`. A second stamp replaces the first —
    /// the pilot's latest order is the one the arena will end on. See
    /// `ArenaRegistry::emcon_hints` for the four ways it ends.
    pub fn hint_emcon(&mut self, pilot: ObjId, on: bool, until_s: i64) {
        self.emcon_hints.insert(pilot, (on, until_s));
    }

    /// **The switch that pilot last forwarded, while the hint stands** at the
    /// world second `now_s` (16b cut O.2), or `None` — no hint, or an expired
    /// one, and then the cockpit reads the mirror. An expired row is left where
    /// it is, as [`ArenaRegistry::starting_hint`] leaves one.
    #[must_use]
    pub fn emcon_hint(&self, pilot: ObjId, now_s: i64) -> Option<bool> {
        self.emcon_hints
            .get(&pilot)
            .filter(|(_, until_s)| now_s <= *until_s)
            .map(|(on, _)| *on)
    }

    /// **Forget one pilot's pod-switch hint** (16b cut O.2) — the confirmed and
    /// the refused ends come through here.
    pub fn clear_emcon_hint(&mut self, pilot: ObjId) {
        self.emcon_hints.remove(&pilot);
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
    /// **17b cut M3.5** (doc 17 §13): a scripted seal's runner could not be
    /// installed — its source is neither compiled in nor in this server's
    /// store, or does not compile. Typed since 17b cut M4.3 (the lobby's
    /// call-off reads the hull); the words are unchanged.
    #[error("a sealed mind cannot be run: {0}")]
    Mind(crate::minds::InstallError),
}

impl From<crate::arena_task::SpawnError> for OpenMatchError {
    fn from(error: crate::arena_task::SpawnError) -> Self {
        match error {
            crate::arena_task::SpawnError::Arena(error) => OpenMatchError::Arena(error),
            crate::arena_task::SpawnError::Mind(error) => OpenMatchError::Mind(error),
        }
    }
}

/// **The sources the scripted seals among `seals` name** (17b cut M3.5; doc
/// 17 §13), keyed by each seal's digest — what
/// `crate::minds::install_sealed_runners` is handed through
/// [`crate::arena_task::spawn_arena`] and [`crate::arena_task::resume_arena`]:
/// the store's text at the seal's `source_sha256`
/// (`crate::store::TacsimStore::mind_source`). A seal whose source the store
/// does not hold is left out, so the installer refuses it by name. `native` and
/// `reflex` seals name nothing to run, so the map is empty on a fresh
/// production check-out — before 17b cut M4.3 (every production seal
/// `reflex` or `native`) and again since the fill decision (2026-10-09,
/// every one `native`); from M4.3 to the decision every one was `scripted`
/// on the stock source, and such a match resumed since still names it.
fn sealed_sources<'a>(
    w: &mush_core::World,
    seals: impl Iterator<Item = &'a crate::arena::MindSeal>,
) -> BTreeMap<[u8; 32], String> {
    // 17b cut M4.3: Standard Doctrine's own text with no store at all.
    let store = w.module_state::<crate::store::TacsimStore>();
    sources_by_seal(seals, |sha256| crate::minds::source_lookup(store, sha256))
}

/// **[`sealed_sources`]' rule over any source lookup** (17b cut M3.5): each
/// scripted seal's text, looked up by its `source_sha256` and KEYED BY THE
/// SEAL'S DIGEST — the only digest the arena holds of a unit's seal
/// (`crate::arena::mind::SealRef::digest`), which the installer asks by.
fn sources_by_seal<'a, 's>(
    seals: impl Iterator<Item = &'a crate::arena::MindSeal>,
    source: impl Fn(&[u8; 32]) -> Option<&'s str>,
) -> BTreeMap<[u8; 32], String> {
    seals
        .filter(|seal| seal.engine == crate::arena::mind::SealEngine::Scripted)
        .filter_map(|seal| source(&seal.source_sha256).map(|text| (seal.digest(), text.to_owned())))
        .collect()
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
    open_task(
        ctx,
        match_id,
        custody,
        (arena_cfg, match_cfg, sensors_cfg),
        None,
    )
}

/// **The longest an IC field may run before the kernel's duration bound ends it** (doc 32 cut
/// CF.2): four hours of wall time. A backstop, never the demotion rule (Q-CF-20: the field
/// disengages); an end by it is committed and demotes like a disengagement.
pub const FIELD_MAX_DURATION_MS: u32 = 14_400_000;

/// **Open one IC field's task and register it** (doc 32 cut CF.2): [`open_match`]'s registry
/// half over [`crate::arena_task::spawn_field`], under the field's
/// [`crate::arena_task::FieldTerms`] and the server's own blocks with
/// [`FIELD_MAX_DURATION_MS`]. No match record is written — the field host keeps its own
/// (`crate::field::record`).
///
/// # Errors
///
/// As [`open_match`]'s.
pub fn open_field_task(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    custody: crate::arena::state::CustodyOut,
    terms: crate::arena_task::FieldTerms,
) -> Result<crate::arena_task::Callsigns, OpenMatchError> {
    let (arena_cfg, mut match_cfg, sensors_cfg) =
        crate::match_::lobby::configs(ctx.w).ok_or(OpenMatchError::Unavailable)?;
    match_cfg.max_duration_ms = FIELD_MAX_DURATION_MS;
    open_task(
        ctx,
        task,
        custody,
        (&arena_cfg, &match_cfg, &sensors_cfg),
        Some(terms),
    )
}

/// [`open_match`] and [`open_field_task`]'s one body.
fn open_task(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    custody: crate::arena::state::CustodyOut,
    (arena_cfg, match_cfg, sensors_cfg): (
        &crate::config::ArenaConfig,
        &crate::config::MatchConfig,
        &crate::config::SensorsConfig,
    ),
    terms: Option<crate::arena_task::FieldTerms>,
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
    // 17b cut M1b.5: who flies each unit at check-out, for `@mind/live`.
    let controllers: BTreeMap<ObjId, Controller> = custody
        .units
        .iter()
        .map(|unit| (unit.id, unit.controller))
        .collect();
    // **17b cut M5.2** (the design note's item 10): each hull's first sealed
    // profile, the card its mind row opens on (`Arena::adopt_mind` over
    // `Arena::active_card`) — Line for a hull nobody chose a mind for.
    let mut first_cards = BTreeMap::new();
    let mut first_catalog = crate::arena::runtime_card::CardCatalog::default();
    for unit in &custody.units {
        let (_, card) = unit.seal.cards.admit(&mut first_catalog).map_err(|error| {
            crate::arena::state::ArenaError::Mind {
                unit: unit.id,
                reason: error.to_string(),
            }
        })?;
        if let Some(stock) = card.stock() {
            first_cards.insert(unit.id, stock.clone());
        }
    }
    let armed: BTreeSet<ObjId> = custody
        .units
        .iter()
        .filter(|unit| !unit.weapons.is_empty())
        .map(|unit| unit.id)
        .collect();
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
    // **17b cut M1b.1** (doc 17 §5, D-UM-17 as amended): the Compact floor, read
    // once here off the server's `[tacsim.minds]` (its default when the block
    // is absent) and captured into the arena by `spawn_arena`; nothing after
    // check-out reads the config's floor again.
    let minds_cfg = crate::match_::lobby::minds_config(ctx.w).unwrap_or_default();
    // **17b cut M3.5** (doc 17 §13): the sources the custody's scripted seals
    // name, off the store — none on a fresh production check-out except from
    // 17b cut M4.3 to the fill decision (2026-10-09), when every production
    // seal was `scripted`.
    let sources = sealed_sources(ctx.w, custody.units.iter().map(|unit| &unit.seal));
    // **17b cut M4.1** (doc 17 §6 "Execution policy", D-UM-22): the process's
    // one think pool, sized once from `[tacsim.minds].workers` by whichever
    // match opens first.
    let pool = crate::minds::MindPool::shared(minds_cfg.workers);
    let (handle, join, callsigns) = match terms {
        None => crate::arena_task::spawn_arena(
            &runtime,
            match_id,
            custody,
            arena_cfg,
            match_cfg,
            sensors_cfg,
            &minds_cfg,
            &sources,
            pool,
            egress,
        )?,
        Some(terms) => crate::arena_task::spawn_field(
            &runtime,
            match_id,
            custody,
            (arena_cfg, match_cfg, sensors_cfg, &minds_cfg),
            &sources,
            pool,
            egress,
            terms,
        )?,
    };
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
    // **17b cut M1b.5**: the floor the arena captured, from the very read
    // `spawn_arena` was handed, and the check-out's controllers.
    if let Some(mirror) = registry.matches.get_mut(&match_id) {
        mirror.odds_floor_permille = minds_cfg.odds_floor_permille;
        // Every mind row opens on its hull's first sealed profile over this
        // doctrine (`MindState::fresh_with`; Line for a hull nobody chose a
        // mind for, 17b cut M5.2) — and from 17b cut M4.3 every hull is
        // sealed, a seated one too (a released seat's mind reads the card),
        // so every hull has one.
        for (unit, card) in first_cards {
            mirror.cards.insert(unit, (card, 0, CardSource::CheckOut));
        }
        mirror.controllers = controllers;
        mirror.armed = armed;
    }
    for unit in units {
        registry.bind_unit(unit, match_id);
    }
    for (pilot, unit, station, epoch) in pilots {
        // **16b cut I.4**: a pilot is never a watcher. Unreachable through the
        // verbs today — `@arena/watch` refuses anybody holding a centre slot,
        // a slot is claimed only while the centre is not busy, the centre runs
        // one match at a time, and `BOARD` needs a slot — so this is ONE
        // defensive unbind per pilot, bookkeeping only (the other match, if
        // there were one, would keep building pictures nobody is sent).
        registry.unbind_watcher(pilot);
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
// Resuming a match (16b cut AF.3; 16c N16)
// ---------------------------------------------------------------------------

/// **Re-open one match from its decoded snapshot and register it** (16b cut
/// AF.3; doc 16 §3.1 "Durability") — [`open_match`] from an
/// [`crate::arena::state::Arena`] the boot decoded rather than a check-out.
///
/// In order: the id must not be live and there must be a runtime; then, read
/// off the arena **before** it moves into its task — every row of its epoch
/// table (the callsigns are not read: the record the live record carried has
/// them already), and the **seated roster** `arena.pilots` as
/// `(pilot, unit)` pairs, which is the answer; then the ledger's
/// `flushed_upto` is set to its last closed chunk (every chunk closed before
/// the snapshot reached the world before it did — the task's ordered backlog —
/// and the boot's phase A checked that the tape reaches it); then the task
/// ([`crate::arena_task::resume_arena`]), the table, the mirror row, the ground,
/// every unit bound and every epoch row published. **No pilot is bound and no
/// watcher**: the resume releases every seat to its sealed mind ("to doctrine"
/// here until 17b cut M5.7), and the pilots come back through `BOARD`.
///
/// The mirror is the snapshot's moment: the chunk counters off the live
/// record, the arena clock and the snapshot's time and hash, and
/// [`MatchMirror::resumed_from_ms`], which the live record written next carries
/// as the resume-once rule's memory.
///
/// # Errors
///
/// [`OpenMatchError`]: no registry, no runtime, or a duplicate id. Nothing is
/// registered on any of them.
pub fn resume_match(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    mut arena: crate::arena::Arena,
    live: &crate::match_::LiveRecord,
) -> Result<Vec<(ObjId, ObjId)>, OpenMatchError> {
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
    let arena_cfg = crate::match_::lobby::configs(ctx.w)
        .map(|(arena_cfg, _, _)| arena_cfg)
        .ok_or(OpenMatchError::Unavailable)?;
    // OOC detachment hosting is process-local. Resume ends its ownership and
    // standing objectives; a new explicit opt-in mints fresh admission fences.
    arena.reset_detachment_commands();
    // Everything the world keeps of the arena, read before the move.
    let recovered_orders = arena.recovered_order_watches();
    let units: Vec<ObjId> = arena.units.iter().map(|unit| unit.id).collect();
    let epochs: Vec<((ObjId, Channel), u64)> = arena.epochs.rows().collect();
    let roster: Vec<(ObjId, ObjId)> = arena
        .pilots
        .iter()
        .map(|pilot| (pilot.pilot, pilot.unit))
        .collect();
    let at_ms = arena.now_ms;
    let state_hash = crate::arena::hash::state_hash(&arena);
    // **17b cut M1b.5**: the resumed arena's OWN floor (captured at its
    // check-out and carried by the snapshot) and its controllers, for
    // `@mind/live` — never the live config's floor.
    let odds_floor_permille = arena.odds_floor_permille;
    let controllers: BTreeMap<ObjId, Controller> = arena
        .units
        .iter()
        .map(|unit| (unit.id, unit.controller))
        .collect();
    let armed: BTreeSet<ObjId> = arena
        .units
        .iter()
        .filter(|unit| !unit.weapons.is_empty())
        .map(|unit| unit.id)
        .collect();
    // 17b cut M2.2: the resumed arena's own `Arc` (`Arena.definition`), shared.
    let ground = Arc::clone(&arena.definition);
    // 16c N16: the snapshot carries no chunk bytes, and every chunk closed
    // before it is durable (the backlog's order, and phase A's tape check), so
    // the resumed ledger owes the world nothing below its last closed chunk.
    arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
    // **17b cut M3.5** (doc 17 §13): the decode left `Arena::runners` empty;
    // every scripted seal's runner is installed again from the sources the
    // record's seals name, keyed by each seal's digest — the decoded
    // `UnitState::seal` must be the record's for a source to be found.
    let sources = sealed_sources(ctx.w, live.record.minds.values().map(|row| &row.seal));
    // 17b cut M4.1: a resumed match thinks on the process's one pool, sized
    // by the server's `[tacsim.minds].workers` when it is the first to open.
    let workers = crate::match_::lobby::minds_config(ctx.w)
        .unwrap_or_default()
        .workers;
    let pool = crate::minds::MindPool::shared(workers);
    let tape_resume = crate::match_::mind_tape::resume(ctx, match_id, arena.tape_cursor);
    let (handle, join) = crate::arena_task::resume_arena_recorded(
        &runtime,
        match_id,
        arena,
        &arena_cfg,
        &sources,
        pool,
        egress,
        tape_resume,
    )
    .map_err(OpenMatchError::Mind)?;
    if let Some((old, old_join)) = tasks.insert(match_id, handle, join) {
        // Cannot happen (the id was checked above); stopped rather than leaked.
        old.signal_stop();
        old_join.abort();
    }
    let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
        if let Some((handle, join)) = tasks.remove(match_id) {
            handle.signal_stop();
            join.abort();
        }
        return Err(OpenMatchError::Unavailable);
    };
    registry.track(match_id);
    registry.restore_order_watches(match_id, recovered_orders);
    registry.arenas.insert(match_id, ground);
    for unit in units {
        registry.bind_unit(unit, match_id);
    }
    for ((unit, channel), epoch) in epochs {
        registry.set_epoch(unit, channel, epoch);
    }
    if let Some(mirror) = registry.matches.get_mut(&match_id) {
        mirror.chunks_appended = live.chunks_appended;
        mirror.chunks_lost = live.chunks_lost;
        mirror.arena_ms = at_ms;
        mirror.now_ms = at_ms;
        mirror.snapshot_at_ms = Some(at_ms);
        mirror.snapshot_hash = Some(state_hash);
        mirror.resumed_from_ms = Some(at_ms);
        mirror.odds_floor_permille = odds_floor_permille;
        mirror.controllers = controllers;
        mirror.armed = armed;
    }
    Ok(roster)
}

/// **Re-open one IC field's task from its decoded snapshot and register it** (doc 32 cut CF.2;
/// Q-CF-33): [`resume_match`]'s registry half over [`crate::arena_task::resume_field`], with no
/// match record and no mind tape. **No pilot is bound** here; the field host rebinds every pilot
/// still sitting in its unit. The resumed ledger owes the world nothing below its last closed
/// chunk, and the host writes this segment's chunks to a log of its own, so no tail is restored.
///
/// # Errors
///
/// [`OpenMatchError`]: no registry, no runtime, a duplicate key, a seal's runner.
pub fn resume_field_task(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    mut arena: crate::arena::Arena,
    terms: crate::arena_task::FieldTerms,
) -> Result<(), OpenMatchError> {
    let tasks = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks.clone())
        .ok_or(OpenMatchError::Unavailable)?;
    if tasks.live_ids().contains(&task) {
        return Err(OpenMatchError::Duplicate(task));
    }
    let egress = tasks.egress_sender().ok_or(OpenMatchError::Unavailable)?;
    let runtime = tokio::runtime::Handle::try_current().map_err(|_| OpenMatchError::NoRuntime)?;
    let arena_cfg = crate::match_::lobby::configs(ctx.w)
        .map(|(arena_cfg, _, _)| arena_cfg)
        .ok_or(OpenMatchError::Unavailable)?;
    arena.reset_detachment_commands();
    let units: Vec<ObjId> = arena.units.iter().map(|unit| unit.id).collect();
    let epochs: Vec<((ObjId, Channel), u64)> = arena.epochs.rows().collect();
    let at_ms = arena.now_ms;
    let state_hash = crate::arena::hash::state_hash(&arena);
    let controllers: BTreeMap<ObjId, Controller> = arena
        .units
        .iter()
        .map(|unit| (unit.id, unit.controller))
        .collect();
    let armed: BTreeSet<ObjId> = arena
        .units
        .iter()
        .filter(|unit| !unit.weapons.is_empty())
        .map(|unit| unit.id)
        .collect();
    let ground = Arc::clone(&arena.definition);
    arena.ledger.mark_flushed(arena.ledger.last_closed_seq());
    let workers = crate::match_::lobby::minds_config(ctx.w)
        .unwrap_or_default()
        .workers;
    let pool = crate::minds::MindPool::shared(workers);
    let (handle, join) = crate::arena_task::resume_field(
        &runtime,
        task,
        arena,
        &arena_cfg,
        &BTreeMap::new(),
        pool,
        egress,
        terms,
    )
    .map_err(OpenMatchError::Mind)?;
    if let Some((old, old_join)) = tasks.insert(task, handle, join) {
        old.signal_stop();
        old_join.abort();
    }
    let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
        if let Some((handle, join)) = tasks.remove(task) {
            handle.signal_stop();
            join.abort();
        }
        return Err(OpenMatchError::Unavailable);
    };
    registry.track(task);
    registry.arenas.insert(task, ground);
    for unit in units {
        registry.bind_unit(unit, task);
    }
    for ((unit, channel), epoch) in epochs {
        registry.set_epoch(unit, channel, epoch);
    }
    if let Some(mirror) = registry.matches.get_mut(&task) {
        mirror.arena_ms = at_ms;
        mirror.now_ms = at_ms;
        mirror.snapshot_at_ms = Some(at_ms);
        mirror.snapshot_hash = Some(state_hash);
        mirror.resumed_from_ms = Some(at_ms);
        mirror.controllers = controllers;
        mirror.armed = armed;
    }
    Ok(())
}

/// **Restore the ledger tails of the logs a boot resumes** (16b cut AF.3) — the
/// door `crate::boot_matches` reaches the registry's private [`Ledger`] by.
///
/// The registry's ledger is fresh at every boot, and nothing restored its
/// per-log tails while no match ever appended to its log again after a
/// restart. A resumed match does: without this its first append would open
/// `log:match.<id>` at chunk 0 and **overwrite** the durable chunk there. So the
/// boot inspects the resumed logs' own documents and restores their tails —
/// `logs` alone, so a hole in a finished match's tape never costs a running
/// one anything.
///
/// # Errors
///
/// [`mush_core::LedgerRestoreError`] when the inspection holds a breach; the
/// ledger is unchanged.
pub(crate) fn restore_ledger_tails(
    w: &mut mush_core::World,
    history: &mush_core::LedgerHistory,
    logs: &std::collections::BTreeSet<String>,
) -> Result<mush_core::LedgerRestoreReport, mush_core::LedgerRestoreError> {
    let Some(registry) = w.module_state_mut::<ArenaRegistry>() else {
        return Ok(mush_core::LedgerRestoreReport::default());
    };
    registry
        .ledger
        .restore_logs_from_history(history, |log| logs.contains(log))
}

/// The seal threshold the registry's ledger appends under — what the boot
/// inspects a resumed log's tail against (16b cut AF.3).
pub(crate) fn ledger_chunk_target(w: &mush_core::World) -> usize {
    w.module_state::<ArenaRegistry>()
        .map_or(mush_core::Ledger::new().chunk_target_bytes(), |registry| {
            registry.ledger.chunk_target_bytes()
        })
}

/// **Prepare the resume marker for one match's log** (16b cut AF.3): the
/// `{"resumed": {...}}` entry, validated and fully built but not applied, so
/// the boot can refuse the resume before anything is written (a tail sealed at
/// its target is the refusal it exists for).
///
/// # Errors
///
/// [`mush_core::LedgerAppendError`], and the ledger is unchanged.
pub(crate) fn prepare_resume_marker(
    w: &mush_core::World,
    match_id: MatchId,
    marker: serde_json::Value,
) -> Result<mush_core::PreparedLedgerAppend, String> {
    let registry = w
        .module_state::<ArenaRegistry>()
        .ok_or_else(|| "the arena is not available on this server".to_owned())?;
    registry
        .ledger
        .prepare_append(&match_log_id(match_id), marker)
        .map_err(|error| error.to_string())
}

/// **Apply a prepared resume marker and queue its write** (16b cut AF.3) on the
/// module's own namespace, in the boot's own action — before the resumed
/// task's first chunk, which reaches the world only through a later drain.
pub(crate) fn append_resume_marker(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    prepared: mush_core::PreparedLedgerAppend,
) {
    let op = {
        let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
            return;
        };
        match registry.ledger.apply_prepared(prepared) {
            Ok((_seq, op)) => op,
            Err(error) => {
                // Nothing touched the registry's ledger between the prepare
                // and here — the boot owns the loop — so this is said and not
                // expected.
                tracing::error!(
                    %match_id,
                    %error,
                    "tacsim: a resumed match's marker became stale between prepare and apply; \
                     its tape carries no resume marker"
                );
                return;
            }
        }
    };
    push_op(ctx.out, op);
}

/// **A match's tape in the order it counts** (16b cut AF.3; 16c N16): every
/// arena chunk entry of `log:match.<id>` that the match still stands by, and
/// every resume marker, each in order.
///
/// `entries` are the log's entries' values in document order: an arena chunk
/// (`{"chunk": n, "bytes", "lines"}`) or a resume marker
/// (`{"resumed": {"from_chunk": N, "at_ms", "restart_ms"}}`). The log is
/// append-only, and a resumed arena writes `N + 1, N + 2 …` again with
/// different bytes after whatever the old process wrote past its snapshot —
/// **any number** of chunks, the old server's abandoned timeline. So, walking
/// in order: a chunk entry is kept; **at a marker, every kept entry whose
/// chunk is above its `from_chunk` is discarded**; after the walk the last
/// write per sequence wins (a defence — nothing produces two), and the result
/// is sorted by sequence. `[1, 2, 3, 4, 5, marker(2), 3']` is `[1, 2, 3']`: a
/// last-write rule alone would have kept the old 4 and 5.
///
/// Answers `(kept chunks, markers)`. An entry that is neither is dropped.
#[must_use]
pub fn tape_in_order(
    entries: &[serde_json::Value],
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let mut kept: Vec<(u64, serde_json::Value)> = Vec::new();
    let mut markers: Vec<serde_json::Value> = Vec::new();
    for entry in entries {
        if let Some(from) = entry
            .get("resumed")
            .and_then(|resumed| resumed.get("from_chunk"))
            .and_then(serde_json::Value::as_u64)
        {
            kept.retain(|(chunk, _)| *chunk <= from);
            markers.push(entry.clone());
        } else if let Some(chunk) = entry.get("chunk").and_then(serde_json::Value::as_u64) {
            kept.push((chunk, entry.clone()));
        }
    }
    let mut by_seq: BTreeMap<u64, serde_json::Value> = BTreeMap::new();
    for (chunk, entry) in kept {
        by_seq.insert(chunk, entry);
    }
    (by_seq.into_values().collect(), markers)
}

/// **The pager's line for one resume marker** (16b cut AF.3): what
/// `@arena/report` prints at the marker's place in its page, or `None` for an
/// entry that is not a marker. 109 characters at a six-digit chunk and
/// eight-digit milliseconds, under the 120-column floor.
#[must_use]
pub fn resume_marker_line(entry: &serde_json::Value) -> Option<String> {
    let resumed = entry.get("resumed")?;
    let from = resumed.get("from_chunk")?.as_u64()?;
    let at_ms = resumed.get("at_ms")?.as_i64()?;
    Some(format!(
        "-- resumed at chunk {from} ({at_ms} ms of arena time); the old server's chunks after \
         {from} are abandoned --"
    ))
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
    /// **No character entitled to order that squad gave the order** (17b cut
    /// L.1; doc 17 §4, D-UM-52): `admit_order`'s refusal — `Order::by` names
    /// nobody `crate::match_::cockpit::command_of` grants that squad's unit, or
    /// a persona that is not the match record's leader of that squad or a unit
    /// outside that squad's roster.
    /// Unreachable from a cockpit, which names the seat's character and only the
    /// units `command_of` gave it; through `CockpitError::Order` it would read
    /// `Refused: no character entitled to order that squad gave that order.`
    #[error("no character entitled to order that squad gave that order")]
    NoCharacter,
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
/// LLM seam's. Each target must first pass [`send_order`]'s current per-unit
/// admission; a refused target contributes no offer. The arena authorises each
/// accepted unit again at apply against its own
/// table: a unit a pilot has since boarded refuses `NotMachine` silently
/// (`NotReflex` until 17b cut M4.6), as today.
///
/// Answers how many units were offered; `Ok(0)` for an empty reach or when
/// every target refuses admission (the cockpit turns either into
/// [`IntentRefusal::Busy`]).
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
        .map_err(|_| ControlRefusal::Gone)?;
    // **17b cut M1b.5**: the mirror's controller moves as `apply_control`
    // moves the arena's row — a live seat is the pilot's, a released one a
    // mind's (17b cut M4.3: every hull, the dormant row a possess left or a
    // fresh one; the reflex controller's before it, a `minded` set on the
    // mirror telling the two apart until then, which nothing read and 17b cut
    // M4.6a deleted).
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>()
        && let Some(mirror) = registry.matches.get_mut(&match_id)
    {
        let now = match state {
            ControlState::Released => Some(Controller::Mind),
            ControlState::Live => Some(Controller::Pilot),
            ControlState::Linkdead => None,
        };
        if let Some(now) = now
            && let Some(held) = mirror.controllers.get_mut(&unit)
        {
            *held = now;
        }
    }
    // EXIT, reclaim and session hooks share this control boundary. Publish any
    // changed command term before the arena applies queued old-term orders.
    crate::detachment::live::sync(ctx);
    flush_detachment_dispatches(ctx);
    Ok(())
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
///
/// **16b cut AF.3** made it `pub(crate)`: a resumed match releases every seat
/// its decoded arena still holds through it (`crate::match_::resume_pending`).
pub(crate) fn control_all_channels(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    state: ControlState,
    pilot: ObjId,
) {
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

/// The single current-world authorization seam for line, feed and UI consumers.
/// A requested unit/count are selectors only. No authority fact comes from a
/// client, a subscription's cached grant or the trace payload itself.
#[must_use]
pub fn mind_traces(
    w: &mush_core::World,
    actor: ObjId,
    unit: ObjId,
    n: Option<usize>,
) -> Option<Vec<crate::arena::author_trace::ThinkTrace>> {
    if n.is_some_and(|n| !(1..=crate::arena::author_trace::THINKS).contains(&n)) {
        return None;
    }
    let registry = w.module_state::<ArenaRegistry>()?;
    let match_id = registry.match_of_unit(unit)?;
    let record = w
        .module_state::<crate::match_::MatchBook>()?
        .get(match_id)?;
    let seal = &record.minds.get(&unit)?.seal;
    if !crate::match_::draft_holdings::current_draft_source_access(
        w.module_state::<mush_roll::RollStore>(),
        &seal.requested.origin,
    ) {
        return None;
    }
    let principal = match seal.principal.as_ref() {
        Some(crate::deliberation::order::CharacterRef::Character { id }) => Some(*id),
        _ => None,
    };
    let owner = match &seal.requested.origin {
        crate::arena::profile_seal::MindOrigin::Draft { character, .. } => Some(*character),
        _ => None,
    };
    let account = w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor));
    let fill_team = record
        .teams
        .iter()
        .find(|team| team.units.contains(&unit) && !team.pilots.iter().any(|(_, own)| *own == unit))
        .map(|team| team.team);
    let current_fill_setter = crate::match_::lobby::centre(w)
        .filter(|centre| centre.match_id == Some(match_id))
        .and_then(|centre| {
            centre.fill_minds.iter().find_map(|fill| {
                if Some(fill.team) != fill_team {
                    return None;
                }
                match fill.by {
                    crate::match_::lobby::FillSetter::Lead(setter)
                        if centre.leads_side(fill.team, setter) =>
                    {
                        Some(setter)
                    }
                    _ => None,
                }
            })
        });
    let access = crate::arena::author_trace::TraceAccess {
        unit,
        actor,
        account,
        connected: w.is_connected(actor) && w.has_live_conns(actor),
        staff: mush_core::perm::has_power(w, actor, mush_core::Powers::SIM_ADMIN)
            || mush_core::perm::has_role(w, actor, mush_core::Role::Royalty),
        sealed_principal: principal,
        sealed_owner: owner,
        sealed_warrant_principal: principal,
        current_fill_setter,
    };
    registry.mirror(match_id)?.traces.read(unit, n, &access)
}

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
            ArenaOut::MindBulkReady => {}
            ArenaOut::Tape { match_id, write } => {
                crate::match_::mind_tape::accept(ctx, match_id, write)
            }
            ArenaOut::MindTraces { match_id, updates } => {
                if let Some(mirror) = ctx
                    .w
                    .module_state_mut::<ArenaRegistry>()
                    .and_then(|registry| registry.matches.get_mut(&match_id))
                {
                    for update in updates {
                        mirror.traces.apply(update);
                    }
                }
            }
            ArenaOut::Frames {
                match_id,
                frames,
                now_ms,
                observer,
            } => on_frames(ctx, match_id, frames, now_ms, observer),
            ArenaOut::Ledger { match_id, chunk } => {
                on_chunk(ctx, match_id, chunk.seq, &chunk.bytes);
            }
            ArenaOut::Lagged { match_id } => on_lagged(ctx, match_id),
            ArenaOut::Deliberate { match_id, signals } => {
                on_deliberate(ctx, match_id, &signals);
            }
            ArenaOut::Snapshot { match_id, snapshot } => on_snapshot(ctx, match_id, snapshot),
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
    crate::minds::bulk::world::drain(ctx);
}

/// One pulse's frames: a keyframe per pilot, on the feed, through the loop's
/// ordinary output path (doc 16 §3.1 — the arena never touches a sink).
///
/// **16b cut D.1**: `now_ms` is the arena's own clock at the end of the step that
/// produced this message, and the message may carry no frame at all — see
/// [`crate::arena_task::ArenaOut::Frames`]. The mirror takes the clock whether or
/// not anybody was told anything, which is what makes the board's `arena time`
/// line move on a match nobody is moving in.
///
/// **16b cut I.4**: a staff picture on the message goes to the match's
/// watchers ([`on_observer`]) FIRST, and touches nothing the pilot loop below
/// reads or writes.
fn on_frames(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    frames: Vec<(ObjId, Frame)>,
    now_ms: i64,
    observer: Option<Observer>,
) {
    if let Some(observer) = observer {
        on_observer(ctx, match_id, observer);
    }
    // Doc 32 cut CF.2 (Q-CF-14): a fire order waiting on its shooter's sensors.
    if crate::field::hosts(ctx.w, match_id) {
        crate::field::on_frames(ctx, match_id, &frames, now_ms);
    }
    let Some(feed) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.frames.clone())
    else {
        return;
    };
    let mut sent = 0_u64;
    let mut tempo = None;
    // **16b cut AP.1** (16c `B-86`): the receipt, read once for the message and
    // not once per pilot — every frame of one pulse was told at one moment.
    let received = Instant::now();
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
        // **16b cut O.2**: the pod-switch hint's first two ends, by the same
        // rule — the frame's `ecm` agreeing with the switch the pilot sent, or
        // the arena's refusal of an `emcon` order.
        let emcon_refused = frame.notices.iter().any(|notice| {
            matches!(
                notice,
                crate::arena::perception::FrameNotice::Refused { order, .. }
                    if order.as_str() == IntentKind::Emcon { on: true }.as_str()
            )
        });
        // **16b cut AH.1** (16c N18): the frame-borne hint points, read off this
        // frame beside the two hints above and for the same reason — BEFORE the
        // mirror is written, while the frame still carries its notices and while
        // `last_frame` still says whether this pilot has had a frame of this
        // match. `None` is a first frame; so is one held from another match, and
        // a reclaim's or a resume's first frame (an unbind drops the held frame,
        // a resume starts a fresh registry), where the doctrine has the power up
        // and `frame_points` leaves the panel point alone.
        let first_of_match = ctx
            .w
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.last_frame(pilot))
            .is_none_or(|held| held.match_id != match_id);
        let points = crate::match_::tutorial::frame_points(first_of_match, &frame);
        // **16b cut AP.3** (16c N19): the line-mode digest, built only for a
        // pilot who typed `WATCH ON` — read beside `first_of_match`, before the
        // mirror is written and while `ranges` is still in hand: the frame as
        // sent, its own weapon rows (`frame_weapons`; no `Instant` is needed
        // here, the frame is that instant's) and the SETTLED ranges, the very
        // figures the keyframe below carries. Pushed after the keyframe, and
        // only if it reads differently from the last block — see below.
        let watch_block = ctx
            .w
            .module_state::<ArenaRegistry>()
            .is_some_and(|registry| registry.line_watching(pilot))
            .then(|| {
                crate::match_::cockpit::watch_lines(
                    &frame,
                    &ranges,
                    &crate::match_::cockpit::frame_weapons(&frame),
                    crate::match_::cockpit::pass_ms(ctx.w),
                )
                .join("\n")
            });
        if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
            if confirmed || refused {
                registry.clear_starting_hint(pilot);
            }
            let emcon_confirmed = registry
                .emcon_hints
                .get(&pilot)
                .is_some_and(|(on, _)| frame.ecm == Some(*on));
            if emcon_confirmed || emcon_refused {
                registry.clear_emcon_hint(pilot);
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
                    // 16b cut AP.1: what the pilot was told, and when.
                    arena_ms: now_ms,
                    received,
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
        crate::detachment::live::sync(ctx);
        flush_detachment_dispatches(ctx);
        let payload = crate::detachment::commands::enrich(ctx.w, pilot, payload);
        feed.keyframe_to(ctx, Audience::Actor(pilot), payload);
        // **16b cut AP.3** (16c N19): the digest, AFTER the keyframe — a panel
        // client composes the frame before it rings anything, and the line
        // client prints the frame's notices and then the block — and BEFORE
        // the AH.1 hints, so a hint stays the last word on its frame.
        //
        // **Only when the block reads differently from the last one pushed.**
        // `Frame::same_as` re-sends a frame for a moved cell or pose, a global
        // tempo flip and a held contact's age at every whole second, none of
        // which the block prints: without the comparison a pilot driving
        // straight would read the same lines twice a second, and the rate they
        // arrived at would read out the arena's tempo, which 16c `T-59` took off
        // `STATUS`. ONE `Sys` of one to three lines, as the catch-up block goes
        // out — at most one event more per watching pilot per changed frame
        // against `Q-U2`'s budget, and in practice one per changed block —
        // tagged as the frame's own sentences, so the TUI's cockpit ring and
        // the web's notices list (which draw the frame already) leave it out
        // and it lands in their scrollback only.
        if let Some(block) = watch_block {
            let changed = ctx
                .w
                .module_state_mut::<ArenaRegistry>()
                .is_some_and(|registry| registry.note_watch_block(pilot, &block));
            if changed {
                ctx.out.sys_to_actor(pilot, NOTICE_SENTENCE_TAG, block);
            }
        }
        // **16b cut AH.1**: the points, AFTER the keyframe and in point order.
        // After, so the frame's own notice lines print above the hint on the
        // line client (`Power online.`, then what to do with it), both panels
        // compose the frame's notices before they ring the hint, and the web's
        // per-match ring reset at a match's first frame (16b cut B.2) cannot
        // swallow the panel hint. Only to a pilot holding the centre's slot for
        // THIS match — see [`holds_centre_slot`].
        if holds_centre_slot(ctx.w, pilot, match_id) {
            for point in points {
                crate::match_::tutorial::hint(ctx, pilot, point);
            }
        }
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

/// **Whether `pilot` holds the simulator centre's slot for this match** (16b cut
/// AH.1) — the one gate on the three frame-borne hint points.
///
/// **Both halves do work, and it is the slot half that keeps a staff seat out.**
/// A staff match from `@arena/start` is a match the centre DOES hold —
/// `lobby::adopt` binds the centre to it, `Running` with `match_id: Some(id)` —
/// and it holds NO slot, so a staff seat passes the match-id test and fails
/// `slot_of_pilot`. A later refactor must not narrow this to a match-id test. A
/// watcher is fed by `on_observer`, a machine has no pilot and a replay never
/// reaches `on_frames`, so none of them asks.
fn holds_centre_slot(w: &mush_core::World, pilot: ObjId, match_id: MatchId) -> bool {
    crate::match_::lobby::centre(w)
        .filter(|centre| centre.match_id == Some(match_id))
        .and_then(|centre| centre.slot_of_pilot(pilot))
        .is_some()
}

/// **One staff picture, to the match's watchers** (16b cut I.4; 16c N10).
///
/// Kept as the match's last picture (what the `arena.observer` query answers a
/// watcher who resyncs) and published as
/// [`crate::match_::observer::observer_frame`]'s keyframe, with the tempo floor
/// the mirror holds, to each watcher of the match — at most [`MAX_WATCHERS`],
/// so at most eight events — **whose object still exists and who is still
/// staff**. The power is re-checked here because a watch outlives the verb
/// that began it: a watcher who lost `SIM_ADMIN` is unwatched with the cleared
/// picture ([`unwatch`]), and one whose object has gone is unbound. Nobody
/// else is sent anything, and no pilot ever is.
fn on_observer(ctx: &mut Ctx<'_>, match_id: MatchId, observer: Observer) {
    let Some((feed, watchers, floor_ms, ground)) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| {
            let ground = registry.arenas.get(&match_id).cloned()?;
            // A picture the arena built before it heard the last watcher
            // leave is kept for nobody and sent to nobody.
            let watchers = registry.watchers_of(match_id);
            if watchers.is_empty() {
                return None;
            }
            registry.last_observer.insert(match_id, observer.clone());
            Some((
                registry.observers.clone(),
                watchers,
                registry
                    .matches
                    .get(&match_id)
                    .and_then(|mirror| mirror.tempo_floor_ms),
                ground,
            ))
        })
    else {
        return;
    };
    let payload =
        crate::match_::observer::observer_frame(match_id, &observer, floor_ms, &ground.terrain);
    for watcher in watchers {
        if ctx.w.get(watcher).is_none() || !crate::match_::lobby::is_staff(ctx.w, watcher) {
            unwatch(ctx, watcher);
            continue;
        }
        feed.keyframe_to(ctx, Audience::Actor(watcher), payload.clone());
    }
}

/// **Tell one watcher's panel there is nothing to watch** (16b cut I.4): one
/// `arena.observer` keyframe carrying
/// [`crate::match_::observer::observer_cleared`], the way [`clear_cockpit`]
/// clears a cockpit. A watcher whose object has gone is sent nothing.
pub fn clear_observer(ctx: &mut Ctx<'_>, watcher: ObjId) {
    let Some(feed) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.observers.clone())
    else {
        return;
    };
    if ctx.w.get(watcher).is_none() {
        return;
    }
    feed.keyframe_to(
        ctx,
        Audience::Actor(watcher),
        crate::match_::observer::observer_cleared(),
    );
}

/// **The match that staffer watches** (16b cut I.4), read off the registry —
/// what `SIM REPLAY` refuses a watcher on.
#[must_use]
pub fn watching(w: &mush_core::World, actor: ObjId) -> Option<MatchId> {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.watching(actor))
}

/// **Begin watching a running match** (16b cut I.4; 16c N10: `@arena/watch
/// <match>`). The verb's staff gate has already run.
///
/// Refuses, in order:
///
/// 1. an actor who holds a centre slot — their own
///    (`Centre::slot_of_pilot`) or any character's of their account
///    (`Centre::slot_of_account`, read the way the lobby's own `claim` reads
///    it) — or a seat binding ([`ArenaRegistry::seat_of_pilot`]), with
///    [`WATCH_FLYING`]. A dropped or `EXIT`ed pilot keeps their slot, so they
///    are refused too. **This is what makes a watcher never a pilot**: the
///    centre runs one match at a time, a slot is claimed only while it is not
///    busy and `BOARD` needs one, so nobody can become a pilot of a running
///    match while watching it;
/// 2. a match the book does not hold, or holds in any state but `running`:
///    `There is no match 12.` / `Match 12 is {state}.`;
/// 3. a ninth watcher: `Match 12 already has 8 watchers.` (an actor already
///    watching it is not counted twice).
///
/// Then the actor's replay cursor is dropped (one picture at a time), the
/// actor is bound — moved off any other match they watched, whose own watch
/// ends as [`unwatch`] ends one — and, for the match's first watcher, the task
/// is offered `ArenaMsg::Watch { on: true }`. A refused offer unbinds again:
/// `That match is busy; try again in a moment.` while the task lives.
///
/// # Errors
///
/// Every refusal above, as the line the staffer sees.
pub fn watch(ctx: &mut Ctx<'_>, actor: ObjId, match_id: MatchId) -> Result<(), CmdError> {
    let account = ctx
        .w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor));
    let slotted = crate::match_::lobby::centre(ctx.w).is_some_and(|centre| {
        centre.slot_of_pilot(actor).is_some()
            || account.is_some_and(|account| centre.slot_of_account(account).is_some())
    });
    let seated = ctx
        .w
        .module_state::<ArenaRegistry>()
        .is_some_and(|registry| registry.seat_of_pilot(actor).is_some());
    if slotted || seated {
        return Err(CmdError::msg(WATCH_FLYING));
    }
    let state = ctx
        .w
        .module_state::<crate::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .map(|record| record.state);
    match state {
        Some(crate::match_::MatchState::Running) => {}
        Some(state) => {
            return Err(crate::match_::ArenaVerbError::WrongState {
                match_id: match_id.0,
                state: state.as_str(),
            }
            .into());
        }
        None => return Err(crate::match_::ArenaVerbError::NoMatch(match_id.0).into()),
    }
    let (already, others, tasks) = {
        let registry = ctx
            .w
            .module_state::<ArenaRegistry>()
            .ok_or(crate::match_::ArenaVerbError::Unavailable)?;
        let watchers = registry.watchers_of(match_id);
        (
            watchers.contains(&actor),
            watchers.len(),
            registry.tasks().clone(),
        )
    };
    if already {
        return Ok(());
    }
    if others >= MAX_WATCHERS {
        return Err(CmdError::msg(format!(
            "Match {match_id} already has {MAX_WATCHERS} watchers."
        )));
    }
    // One picture at a time: a watch closes the actor's replay.
    if let Some(book) = ctx.w.module_state_mut::<crate::match_::MatchBook>() {
        book.drop_replay_cursor(actor);
    }
    // Moving from another match ends that watch as `off` would.
    if watching(ctx.w, actor).is_some() {
        unwatch(ctx, actor);
    }
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.bind_watcher(actor, match_id);
    }
    if others == 0 && tasks.offer(match_id, ArenaMsg::Watch { on: true }).is_err() {
        if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
            registry.unbind_watcher(actor);
        }
        return Err(CmdError::msg(if tasks.is_live(match_id) {
            "That match is busy; try again in a moment."
        } else {
            "That match has ended."
        }));
    }
    Ok(())
}

/// **Stop watching** (16b cut I.4), answering the match that was watched, or
/// `None` for an actor who watched nothing.
///
/// [`ArenaRegistry::unbind_watcher`], then the cleared picture on the actor's
/// panel ([`clear_observer`]), then — when nobody is left watching the match —
/// `ArenaMsg::Watch { on: false }` to its task. A refused offer is logged and
/// nothing else: the arena keeps building its once-a-second picture until the
/// next `Watch` reaches it, and the world sends it to nobody.
///
/// Called by `@arena/watch off`, by `SeatFacts::disconnected` (so the fan-out
/// stays bounded by connected staff), by `on_observer` and the
/// `arena.observer` answer's next frame for a watcher who lost the power, and
/// by a re-bind onto another match. [`crate::match_::finish`] does not call it:
/// the match is ending, so it unbinds each watcher itself before `untrack` and
/// clears each panel after.
pub fn unwatch(ctx: &mut Ctx<'_>, actor: ObjId) -> Option<MatchId> {
    let match_id = ctx
        .w
        .module_state_mut::<ArenaRegistry>()?
        .unbind_watcher(actor)?;
    clear_observer(ctx, actor);
    let (left, tasks) = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map_or((0, None), |registry| {
            (
                registry.watchers_of(match_id).len(),
                Some(registry.tasks().clone()),
            )
        });
    if left == 0
        && let Some(tasks) = tasks
        && tasks.is_live(match_id)
        && tasks
            .offer(match_id, ArenaMsg::Watch { on: false })
            .is_err()
    {
        tracing::warn!(
            %match_id,
            "the last watcher left but the arena task would not take the Watch; it keeps \
             building its picture until the next Watch reaches it, and nobody is sent it"
        );
    }
    Some(match_id)
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

/// **The ground one tracked match is fought over, as the registry's own
/// `Arc`** (16b cut I.1): a clone of the one pointer check-out took (16b cut
/// K.2), so [`crate::match_::finish`] can keep the ground for the match's
/// replay after [`ArenaRegistry::untrack`] drops the registry's row — without
/// rebuilding sixty-five thousand cells and a horizon index. `None` for a match
/// the registry does not track.
#[must_use]
pub fn ground_of(
    w: &mush_core::World,
    match_id: MatchId,
) -> Option<Arc<crate::arena::state::ArenaDefinition>> {
    w.module_state::<ArenaRegistry>()
        .and_then(|registry| registry.arenas.get(&match_id).cloned())
}

/// The ledger log id one match streams into: `log:match.<id>` is
/// `chunk_key("match.<id>", n)`, so the log id itself is `match.<id>`.
#[must_use]
pub fn match_log_id(match_id: MatchId) -> String {
    format!("match.{}", match_id.0)
}

/// **The `mind_card` and `mind_card_refused` lines of one chunk, onto the
/// mirror** (17b cut M1b.5; [`MatchMirror::cards`]): in line order, a
/// `mind_card` writes `(card, t_ms, OnTape)` unless the entry is a `Sent` one
/// newer than the line; a `mind_card_refused` removes a `Sent` entry it is not
/// older than. Per chunk, never per pulse. Since 17b cut M5.2 a seat's `MIND
/// PROFILE` journals a `mind_card` too, so the mirror's card follows a
/// pilot's switch — the card the hull's mind will fly at the release.
fn note_cards_from_tape(ctx: &mut Ctx<'_>, match_id: MatchId, events: &[LedgerEvent]) {
    if !events.iter().any(|event| {
        matches!(
            event.kind,
            EventKind::MindCard { .. } | EventKind::MindCardRefused { .. }
        )
    }) {
        return;
    }
    let Some(mirror) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.matches.get_mut(&match_id))
    else {
        return;
    };
    for event in events {
        let sent_after = |mirror: &MatchMirror, unit: ObjId| {
            mirror
                .cards
                .get(&unit)
                .is_some_and(|(_, at, source)| *source == CardSource::Sent && *at > event.t_ms)
        };
        let sent = |mirror: &MatchMirror, unit: &ObjId| {
            mirror
                .cards
                .get(unit)
                .is_some_and(|(_, _, source)| *source == CardSource::Sent)
        };
        match &event.kind {
            EventKind::MindCard { unit, stock, .. } if !sent_after(mirror, *unit) => {
                if let Some(card) = stock {
                    mirror
                        .cards
                        .insert(*unit, (card.clone(), event.t_ms, CardSource::OnTape));
                } else {
                    mirror.cards.remove(unit);
                }
            }
            EventKind::MindCardRefused { unit, .. }
                if !sent_after(mirror, *unit) && sent(mirror, unit) =>
            {
                mirror.cards.remove(unit);
            }
            _ => {}
        }
    }
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
    // Doc 32 cut CF.2: a field's black box is its own log, never `log:match.<id>`.
    let log_id =
        crate::field::log_of_task(ctx.w, match_id).unwrap_or_else(|| match_log_id(match_id));
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
            // **16b cut I.1**: and the replay says it is not whole.
            if let Some(replay) = record.replay.as_mut() {
                replay.lost_chunk();
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
    // **17b cut M1b.5**: `@mind/live`'s read-back is written from the tape — a
    // `mind_card` line is the card the arena applied, whole, at its
    // millisecond. A `Sent` entry newer than the line (a later offer still on
    // its way) is kept; a `mind_card_refused` line drops a `Sent` entry it is
    // not older than (the offer it answers changed nothing, so the world no
    // longer knows the card: the read-back then says to read the tape).
    note_cards_from_tape(ctx, match_id, &events);
    // **17b cut M3.4** (doc 17 §8; D-UM-32): a hull dropped to the ROM is told
    // to its warrant principal alone, in words read off the tape's own
    // `mind_dropped` line — never off the live config (16b cut V.3's rule).
    say_mind_dropped(ctx, match_id, &events);
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
        // **16b cut I.1 (16c N9, Q37)**: and the replay fold, after them — the
        // same events in the same order, of which it reads only the objective
        // record (positions, shots, wrecks, the end).
        if let Some(replay) = record.replay.as_mut() {
            for event in &events {
                replay.absorb(event);
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
                // **16b cut I.1**: the replay fold has already absorbed this
                // chunk's events above and is left as it is. The replay is a
                // picture of what passed the world, not of what the store kept:
                // the arena journalled these lines and the match happened as they
                // say, so a page the log refused is not a gap in what anybody can
                // be shown.
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

/// **One snapshot of a running match** (16b cut AF.2; 16c N16): the arena's
/// document to `tacsim:snapshot:<n>` and the world's live record to
/// `tacsim:live:<n>`, **in this one action**, so both ride one batch of the
/// arena writer lane and a store that applied one without the other is the
/// exception 16b cut AF.3's moment check catches, not the rule.
///
/// The mirror takes the snapshot's moment and hash first, so the live record
/// written beside it carries `at_ms` equal to the snapshot's own. The document
/// is taken out of the [`Arc`] without a copy in the common case — the task
/// dropped its clone once the egress took the message — and cloned only if
/// something else still holds one. **No ack**: a document is not a chunk, and
/// the task waits on nothing it sends here. A snapshot of a match the book does
/// not hold live (an end already processed, a staff reset) is dropped with a
/// `debug` line: its result has deleted, or is about to delete, both documents.
fn on_snapshot(ctx: &mut Ctx<'_>, match_id: MatchId, snapshot: Arc<crate::arena::Snapshot>) {
    // Doc 32 cut CF.2: a field's snapshot is its host's, beside its live record.
    if crate::field::hosts(ctx.w, match_id) {
        crate::field::on_snapshot(ctx, match_id, snapshot);
        return;
    }
    let live = ctx
        .w
        .module_state::<crate::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .is_some_and(|record| record.state.is_live());
    if !live {
        tracing::debug!(
            %match_id,
            arena_ms = snapshot.at_ms,
            "a snapshot arrived for a match the world no longer holds live; it is dropped"
        );
        return;
    }
    if let Some(mirror) = ctx
        .w
        .module_state_mut::<ArenaRegistry>()
        .and_then(|registry| registry.matches.get_mut(&match_id))
    {
        mirror.snapshot_at_ms = Some(snapshot.at_ms);
        mirror.snapshot_hash = Some(snapshot.state_hash);
    }
    let document = Arc::try_unwrap(snapshot).map_or_else(
        |shared| shared.document.clone(),
        |snapshot| snapshot.document,
    );
    push_op(
        ctx.out,
        mush_core::ModuleWrite::JsonSet {
            key: crate::match_::snapshot_key(match_id),
            value: document,
        },
    );
    crate::match_::write_live(ctx, match_id);
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
    // Doc 32 cut CF.2: a field's end is its host's demotion, never a match's finish.
    if crate::field::hosts(ctx.w, match_id) {
        crate::field::close::on_ended(ctx, match_id, end, units);
        return;
    }
    say_due_orders(ctx, Some(match_id));
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
/// in the tree renders as — since 16b cut AF.1 the arena library's own
/// ([`crate::arena::snapshot::digest_text`]), which the snapshot's head writes
/// its two digests with; re-exported here so every caller is unchanged.
pub use crate::arena::snapshot::digest_text;

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
/// | the grace expires | `Control { Released }` | is flown by its sealed mind (below) |
/// | a clean connect | `Control { Live }` | (a seat that was never released) |
///
/// **16b cut Y.3** added the fifth fact, which is not a hook: past the grace the
/// tank is flown by its sealed mind (the reflex controller until 17b cut M4.3)
/// and the board reads `flown by its mind` (`flown by doctrine` until 17b cut
/// M5.3); a reconnect is told so, and `BOARD` in the running match sends
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
    /// flying on its mind and `BOARD` takes it back (`match_::mind_line`; on
    /// doctrine until 17b cut M5.3) — because before this cut a
    /// reconnecting pilot past the grace was told nothing at all. The tank is
    /// the pilot's own, which they may always be told about (doc 16 §1); what
    /// it did while they were gone is not said here.
    ///
    /// **16b cut AF.3**: a pilot whose seat a boot's resume released
    /// (`Slot::released_by_resume`) reads `match_::RESUME_LINE` before it — once:
    /// while the match book still owes them the pair
    /// (`MatchBook::resume_lines_deferred`) the boarding tick is the one that
    /// says them, and this says nothing; whichever path says the resume line
    /// clears `released_by_resume`, so a login after it reads Y.3's line alone.
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            control_all_channels(ctx, unit, ControlState::Live, actor);
            return;
        }
        let dropped = crate::match_::lobby::centre(ctx.w)
            .filter(|centre| centre.state == crate::match_::MatchState::Running)
            .and_then(|centre| centre.slot_of_pilot(actor))
            .filter(|slot| slot.is_dropped())
            .and_then(|slot| slot.unit.map(|tank| (tank, slot.released_by_resume)));
        let Some((tank, resumed)) = dropped else {
            return;
        };
        // **16b cut AF.3**: a pilot whose seat the boot's resume released is
        // owed the resume line first — but once. While the book still holds the
        // line deferred for them, the boarding tick is the one that says both
        // (and clears the panel a surviving session still shows), so this says
        // nothing; after the tick has dropped it, a later login says them here.
        let deferred = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .is_some_and(|book| book.resume_lines_deferred(actor));
        if deferred {
            return;
        }
        if resumed {
            ctx.out
                .sys_to_actor(actor, NOTICE_CATEGORY, crate::match_::RESUME_LINE);
            // Once: a login after this one reads Y.3's line alone.
            crate::match_::lobby::resume_line_said(ctx, actor);
        }
        ctx.out.sys_to_actor(
            actor,
            NOTICE_CATEGORY,
            crate::match_::mind_line(ctx.w, tank),
        );
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
        crate::detachment::live::sync(ctx);
        flush_detachment_dispatches(ctx);
    }

    /// **16b cut K.2 (K4)**: the pilot's line dropped and the grace is running.
    /// Not a release: the tank keeps steering and stops shooting.
    fn linkdead(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            control_all_channels(ctx, unit, ControlState::Linkdead, actor);
        }
        crate::detachment::live::sync(ctx);
        flush_detachment_dispatches(ctx);
    }

    /// The grace ran out, or the pilot quit: the seat is released and the tank
    /// goes to its sealed mind (the reflex controller until 17b cut M4.3;
    /// re-pointed by 17b cut M5.7).
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
        // **16b cut I.4**: a watch ends with its watcher's connection, so the
        // pictures a pulse fans out go only to staff who are here. A watcher
        // holds no seat, so the seated arm below never runs for one.
        if watching(ctx.w, actor).is_some() {
            unwatch(ctx, actor);
        }
        if let Some(unit) = SeatFacts::seated_unit(ctx, actor) {
            let now_ms = ctx.w.now().saturating_mul(1_000);
            crate::match_::lobby::slot_dropped(ctx, actor, now_ms);
            control_all_channels(ctx, unit, ControlState::Released, actor);
        }
        crate::detachment::live::sync(ctx);
        flush_detachment_dispatches(ctx);
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

/// **`@arena/status`'s `leaders:` line for one match** (17b cut L.2; D-UM-53):
/// the record's [`crate::deliberation::LanceLeader`] rows, the sim personas
/// counted rather than named (`  leaders: 2 sim personas`, `1 sim persona`) and
/// each character leader named by squad and current object name (`, red-1 Ada`,
/// `?` once the object is gone); `  leaders: none` when the record holds no row
/// (or no record is booked). Counted so the line stays short at any squad count
/// — 32 squads a side named one by one would be wider than the 120-column floor.
fn leaders_row(w: &mush_core::World, match_id: MatchId) -> String {
    let Some(record) = w
        .module_state::<crate::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
    else {
        return "  leaders: none".to_owned();
    };
    let personas = record
        .leaders
        .iter()
        .filter(|leader| leader.by.is_persona())
        .count();
    let mut parts: Vec<String> = Vec::new();
    match personas {
        0 => {}
        1 => parts.push("1 sim persona".to_owned()),
        n => parts.push(format!("{n} sim personas")),
    }
    for leader in &record.leaders {
        if let CharacterRef::Character { id } = leader.by {
            let name = w
                .get(id)
                .map_or_else(|| "?".to_owned(), |object| object.name.to_string());
            parts.push(format!("{} {name}", leader.squad));
        }
    }
    if parts.is_empty() {
        "  leaders: none".to_owned()
    } else {
        format!("  leaders: {}", parts.join(", "))
    }
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
        // **17b cut L.2** (R-C): a mind never orders; the deliberated orders are
        // the lance leaders'.
        rows.push(format!(
            "  orders: {human} by pilots, {machine} by lance leaders"
        ));
        // **17b cut L.2** (D-UM-53): who leads the match's squads, on a line of
        // its own for the same reason — staff-only by the verb's gate.
        rows.push(leaders_row(ctx.w, match_id));
        // **16b cut I.3**: the staff floor, on a line of its own for the same
        // reason, and only while one stands.
        if let Some(floor_ms) = mirror.tempo_floor_ms {
            rows.push(format!("  tempo floor: {floor_ms} ms"));
        }
        // **16b cut AF.2**: the last snapshot the world wrote, on a line of its
        // own for the same reason, once there is one. Staff-only, as the verb
        // is: an arena millisecond names nothing on the field.
        if let Some(at_ms) = mirror.snapshot_at_ms {
            rows.push(format!("  last snapshot: at {at_ms} ms of arena time"));
        }
        // **16b cut AF.3**: and, for a match this boot resumed, the moment it
        // resumed from — staff-only by the verb's gate, for the reason above.
        if let Some(at_ms) = mirror.resumed_from_ms {
            rows.push(format!(
                "  resumed: from a snapshot at {at_ms} ms of arena time"
            ));
        }
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

/// **The ground of the match a watcher watches** (16b cut I.4), for
/// `arena.map`'s second clause — only while the watcher is still staff, the
/// same test [`on_observer`] re-checks.
fn watched_ground(
    w: &mush_core::World,
    actor: ObjId,
) -> Option<(MatchId, &crate::arena::state::ArenaDefinition)> {
    let registry = w.module_state::<ArenaRegistry>()?;
    let match_id = registry.watching(actor)?;
    if !crate::match_::lobby::is_staff(w, actor) {
        return None;
    }
    Some((match_id, registry.arena_of(match_id)?))
}

/// **`arena.map`'s handler** (16b cut Q.3): the terrain of the arena the asker
/// is flying in, or [`crate::match_::cockpit::map_unseated`].
///
/// When `params` carries `{ "draft": "<arena_id>" }` (doc 30 §8 & §11 WG.4) and
/// `actor` is staff (`crate::match_::lobby::is_staff`), answers the
/// [`crate::ground::GroundDraft`] reconstructed from the world's content store.
/// Otherwise `params` is ignored today. A `window` parameter is where doc 16 §11's 10 km
/// path would slice it — `{ "window": [x0, y0, w, h] }` in cells — and the
/// payload's `origin_cell` is in it for exactly that: a renderer written against
/// that key keeps working when the map stops starting at the arena's corner.
///
/// **16b cut I.1 gave it an order**, because an unparameterised query
/// can only be keyed on who asks: (1) a seated pilot is answered their own
/// match's map, as before; (2) a watcher, the watched match's ground (16b cut
/// I.4); (3) an actor whose replay cursor names a finished match the ring
/// remembers with a replay, **and whose ground the book holds**, that ground
/// under that match's id; (4) everybody else the unseated pair. It only
/// reads: a ground not yet built after a boot is built by `SIM REPLAY`
/// (`crate::match_::replay::ground_for`), so the first `SIM REPLAY` of a
/// remembered match after a boot is what makes its map answerable. The map is
/// still pulled and never pushed.
/// Parse an optional `"window"` parameter on `arena.map`: either `[x0, y0, w, h]`
/// or `{"x": x0, "y": y0, "width": w, "height": h}` (doc 16 §11; doc 30 §7,
/// §11 WG.6; `D-30-7`, `Q-30-6`).
fn parse_map_window(params: &serde_json::Value) -> Option<(u16, u16, u16, u16)> {
    let win = params.get("window")?;
    if let Some(arr) = win.as_array()
        && arr.len() == 4
    {
        let x0 = u16::try_from(arr[0].as_u64()?).ok()?;
        let y0 = u16::try_from(arr[1].as_u64()?).ok()?;
        let w = u16::try_from(arr[2].as_u64()?).ok()?;
        let h = u16::try_from(arr[3].as_u64()?).ok()?;
        return Some((x0, y0, w, h));
    }
    if let Some(obj) = win.as_object() {
        let x0 = u16::try_from(obj.get("x").or_else(|| obj.get("x0"))?.as_u64()?).ok()?;
        let y0 = u16::try_from(obj.get("y").or_else(|| obj.get("y0"))?.as_u64()?).ok()?;
        let w = u16::try_from(obj.get("width").or_else(|| obj.get("w"))?.as_u64()?).ok()?;
        let h = u16::try_from(obj.get("height").or_else(|| obj.get("h"))?.as_u64()?).ok()?;
        return Some((x0, y0, w, h));
    }
    None
}

/// Build an `arena.map` answer for `ground`, honoring an explicit `window` when
/// supplied or focusing on `focus` (the pilot's cell or the arena's centre) when
/// a 512×512 or steep 256×256 ground needs windowing to stay within
/// [`crate::match_::cockpit::MAP_MAX_BYTES`] (doc 30 §7, §11 WG.6; `D-30-7`).
fn map_answer(
    match_id: MatchId,
    ground: &crate::arena::state::ArenaDefinition,
    window: Option<(u16, u16, u16, u16)>,
    focus: Option<(u16, u16)>,
) -> serde_json::Value {
    match window {
        Some(win) => crate::match_::cockpit::map_frame_window_bounded(
            match_id,
            ground.id.as_str(),
            &ground.terrain,
            &ground.key_spots,
            win,
        ),
        None => crate::match_::cockpit::map_frame_for_focus(
            match_id,
            ground.id.as_str(),
            &ground.terrain,
            &ground.key_spots,
            focus,
        ),
    }
}

struct MapQuery;

impl mush_core::QueryHandler for MapQuery {
    fn query(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        if let Some(draft_id) = params.get("draft").and_then(serde_json::Value::as_str) {
            if !crate::match_::lobby::is_staff(w, actor) {
                return Ok(mush_proto::OutputEvent::Panel {
                    feed: SmolStr::new_static(MAP_FEED),
                    kind: mush_proto::PanelFrameKind::Keyframe,
                    data: serde_json::json!({
                        "schema": crate::ground::DRAFT_SCHEMA,
                        "error": "Permission denied.",
                    }),
                });
            }
            let data = crate::ground::draft_from_world(w, draft_id)
                .and_then(|draft| {
                    serde_json::to_value(draft)
                        .map_err(|e| crate::ground::GroundError::Json(e.to_string()))
                })
                .unwrap_or_else(|e| {
                    serde_json::json!({
                        "schema": crate::ground::DRAFT_SCHEMA,
                        "error": e.to_string(),
                    })
                });
            return Ok(mush_proto::OutputEvent::Panel {
                feed: SmolStr::new_static(MAP_FEED),
                kind: mush_proto::PanelFrameKind::Keyframe,
                data,
            });
        }
        let requested_window = parse_map_window(params);
        let data = match seated_ground(w, actor) {
            // (1) 16b cut V.1: the ground's named spots ride the map, because
            // they are surveyed ground exactly as the terrain is.
            Some((match_id, ground)) => {
                let focus = w
                    .module_state::<ArenaRegistry>()
                    .and_then(|reg| reg.last_frame(actor))
                    .and_then(|held| held.frame.cell);
                map_answer(match_id, ground, requested_window, focus)
            }
            // (2) **16b cut I.4**: a watcher (still staff) is answered the
            // ground of the match they watch, under that match's id.
            None => match watched_ground(w, actor) {
                Some((match_id, ground)) => map_answer(match_id, ground, requested_window, None),
                // (3) 16b cut I.1: the finished match the actor's replay cursor
                // names, drawn on the ground it was fought on.
                None => match crate::match_::replay::cursor_ground(w, actor) {
                    Some((match_id, ground)) => {
                        map_answer(match_id, &ground, requested_window, None)
                    }
                    // (4) Nothing about any match: not its id, not its ground,
                    // not whether one is running at all.
                    None => crate::match_::cockpit::map_unseated(),
                },
            },
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
                crate::detachment::commands::enrich(
                    w,
                    actor,
                    crate::match_::cockpit::status_frame_held(
                        held.match_id,
                        &held.frame,
                        &held.ranges,
                    ),
                )
            });
        Ok(mush_proto::OutputEvent::Panel {
            feed: SmolStr::new_static(FRAME_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data,
        })
    }
}

/// **`arena.replay`'s handler** (16b cut I.1; 16c N9, Q37): the whole replay
/// document of the finished match the asker's replay cursor names, or
/// `available: false` with the sentence that says why not
/// ([`crate::match_::replay::replay_answer`]).
///
/// **It takes no parameters**: the replay cursor names the match (a
/// `Query::PanelSubscribe` may carry parameters since doc 20 cut SM.9, and this
/// handler reads none). So the
/// match is the one the asker last opened with `SIM REPLAY` — which is also why
/// a refused `SIM REPLAY` drops the cursor, so this never answers an earlier
/// match's document by accident. The answer's shape is `replay`, not `grid`:
/// a line client prints nothing for it.
struct ReplayQuery;

impl mush_core::QueryHandler for ReplayQuery {
    fn query(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        Ok(mush_proto::OutputEvent::Panel {
            feed: SmolStr::new_static(REPLAY_FEED),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data: crate::match_::replay::replay_answer(w, actor),
        })
    }
}

/// **`arena.observer`'s handler** (16b cut I.4; 16c N10): the last staff
/// picture of the match the asker watches, re-rendered with the mirror's tempo
/// floor — what a watcher's panel re-asks after a reconnect or a server-half
/// restart — or [`crate::match_::observer::observer_cleared`] to everybody
/// else: a bystander, a pilot, a watcher whose match has sent no picture yet,
/// and a watcher who is no longer staff.
///
/// **A query reads and cannot unbind** (it has the world immutably): a watcher
/// who lost the power is answered `watching: false` here and unwatched by the
/// next picture of their match ([`on_observer`]) — which comes at most once an
/// arena second but only once a pulse, so up to `quiet_ms` (5,000 by default)
/// later at `Quiet` and up to [`crate::arena::MAX_TEMPO_FLOOR_MS`] (10,000)
/// under a staff floor, while they still count against [`MAX_WATCHERS`] — and
/// is sent nothing in between, because that is where the power is re-checked.
/// Takes no parameters, as every query here.
struct ObserverQuery;

impl mush_core::QueryHandler for ObserverQuery {
    fn query(
        &self,
        w: &mush_core::World,
        actor: ObjId,
        _params: &serde_json::Value,
    ) -> Result<mush_proto::OutputEvent, mush_proto::RichText> {
        let data = w
            .module_state::<ArenaRegistry>()
            .filter(|_| crate::match_::lobby::is_staff(w, actor))
            .and_then(|registry| {
                let match_id = registry.watching(actor)?;
                let observer = registry.last_observer(match_id)?;
                let ground = registry.arena_of(match_id)?;
                let floor_ms = registry
                    .mirror(match_id)
                    .and_then(|mirror| mirror.tempo_floor_ms);
                Some(crate::match_::observer::observer_frame(
                    match_id,
                    observer,
                    floor_ms,
                    &ground.terrain,
                ))
            })
            .unwrap_or_else(crate::match_::observer::observer_cleared);
        Ok(mush_proto::OutputEvent::Panel {
            feed: SmolStr::new_static(OBSERVER_FEED),
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
    minds_cfg: &crate::config::MindsConfig,
) -> Result<(), ModError> {
    let frames = reg.panel_feed(FRAME_FEED)?;
    // **16b cut Q.3**: the terrain feed beside it, and its handle dropped at
    // once — nothing ever pushes a map. Registering the feed is what advertises
    // `arena.map` in the client manifest and what routes a `PanelSubscribe` to
    // the query handler below. The order of these two registrations is the order
    // `mush-server`'s boot-parity test pins the manifest in.
    drop(reg.panel_feed(MAP_FEED)?);
    // **16b cut I.1**: the replay feed after it, its handle dropped as the
    // map's is — `SIM REPLAY` pushes its grid through the ordinary output path,
    // exactly as `VIEW` pushes its own — and the manifest order moves with it.
    drop(reg.panel_feed(REPLAY_FEED)?);
    // **16b cut I.4**: the staff picture's feed after it, its handle KEPT on
    // the registry — this one is pushed, to watchers — and the manifest order
    // moves with it.
    let observers = reg.panel_feed(OBSERVER_FEED)?;
    crate::minds::trace_feed::register(reg)?;
    reg.query_handler(MAP_FEED, Arc::new(MapQuery))?;
    reg.query_handler(REPLAY_FEED, Arc::new(ReplayQuery))?;
    reg.query_handler(OBSERVER_FEED, Arc::new(ObserverQuery))?;
    reg.query_handler(FRAME_FEED, Arc::new(CockpitQuery))?;
    reg.async_completion(Arc::new(ArenaEgress))?;
    // **16b cut V.3 (16c N2)**: the tacsim module's FIRST store-read completion,
    // and its only one — `mush_core::Registrar::store_read_completion` takes one
    // per module. It is the re-entry of both deferred reads the debrief makes:
    // `SIM REPORT`'s one report document and `@arena/report`'s page of the tape
    // (`crate::match_::report`).
    reg.store_read_completion(Arc::new(crate::minds::bulk::bench_service::Reads))?;
    reg.hook(mush_core::HookPoint::Connected, Arc::new(SeatFacts));
    // 16b cut K.2 (K4): the kernel's new point, and the revival that rides
    // `Connected`'s registrations above on `EventHook::revived`.
    reg.hook(mush_core::HookPoint::Linkdead, Arc::new(SeatFacts));
    reg.hook(mush_core::HookPoint::Disconnected, Arc::new(SeatFacts));
    let tasks = ArenaTasks::new(egress_capacity);
    let egress = tasks
        .egress_sender()
        .ok_or_else(|| ModError::msg("new arena task egress is unavailable"))?;
    let bulk_cfg = minds_cfg.bulk();
    // The maintenance pulse: production's second (`Coarse`), or the
    // sub-second pace the acceptance stories' harness sets (`Fast`, the
    // kernel's name for a sub-second period; the loop arms both alike).
    let maintenance = Duration::from_millis(bulk_cfg.maintenance_ms);
    let maintenance_cadence = if maintenance >= Duration::from_secs(1) {
        mush_core::TickCadence::Coarse(maintenance)
    } else {
        mush_core::TickCadence::Fast(maintenance)
    };
    let bulk = crate::minds::bulk::world::BulkService::new(bulk_cfg.clone(), egress.clone())
        .map_err(|e| ModError::msg(e.to_string()))?;
    reg.state(
        crate::minds::bulk::bench_service::BenchService::new(bulk_cfg.clone(), egress)
            .map_err(|e| ModError::msg(e.to_string()))?,
    );
    drop(reg.panel_feed(crate::minds::bulk::bench_service::FEED)?);
    reg.state(bulk);
    reg.state(
        crate::minds::bulk::owner::BulkStore::new(bulk_cfg)
            .map_err(|e| ModError::msg(e.to_string()))?,
    );
    reg.state(crate::minds::bulk::artifacts::Artifacts::default());
    drop(reg.panel_feed(crate::minds::bulk::query::FEED)?);
    reg.query_handler(
        crate::minds::bulk::query::FEED,
        Arc::new(crate::minds::bulk::query::BulkQuery),
    )?;
    reg.tick(
        "mind-bulk-maintenance",
        maintenance_cadence,
        Arc::new(
            |ctx: &mut Ctx<'_>, _: &str, _: Option<mush_core::SceneId>| {
                crate::minds::bulk::world::drain(ctx)
            },
        ),
    )?;
    reg.off_loop_simulation_host(Arc::new(tasks.clone()))?;
    reg.state(ArenaRegistry::new(tasks, frames, observers));
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
    reg.command(CommandSpec{
        name:"bench".into(),shape:ArgShape::Raw,
        help:"BENCH LOAD <run>/<game> | LOAD MATCH <match> <callsign>; STEP [n]; RUN TO <minutes:seconds>; BACK [n]; PIC; TRACE [n]; WATCH <expr>|OFF; EVAL <expr>; CLOSE. Private connection-bound replay; expressions read pic, card and mem with a separate500-op ceiling.".into(),
        ..CommandSpec::default()
    },Arc::new(crate::minds::bulk::bench_service::command))?;
    let (arena_cfg, match_cfg, sensors_cfg) =
        (arena_cfg.clone(), match_cfg.clone(), sensors_cfg.clone());
    // One bounded author pool per composition, independent of the arena's live
    // mind workers. Registration failure drops the service and signals shutdown.
    let pictures = Arc::new(crate::minds::check::AuthorPictures::new(
        arena_cfg.clone(),
        minds_cfg.clone(),
    ));
    let executor = crate::minds::workshop::executor::WorkshopExecutor::new(pictures)
        .map_err(|error| ModError::msg(error.to_string()))?;
    let floor = u16::try_from(minds_cfg.odds_floor_permille)
        .map_err(|_| ModError::msg("authoring Compact floor is out of range"))?;
    let workshop = Arc::new(crate::minds::workshop::commands::WorkshopCommands::new(
        executor,
        arena_cfg.clone(),
        floor,
    ));
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
                   read too. A read.\n\
                   @arena/tempo <match>=<period_ms> | <match>=off | <match>   (staff) floor \
                   the wake interval of a running match, in steps of the engaged period up \
                   to 10 s: the arena wakes once per floor of wall time and moves at most \
                   its catch-up clamp of its own tempo per wake, which is slow motion when \
                   the floor is longer; pilots' orders wait for the next wake. Never a \
                   pause, not hashed, recorded on the tape. `=off` lifts it; bare reads it.\n\
                   @arena/place <unit>=<x>,<y>         (staff) put a unit of a running match \
                   on an arena cell. The world refuses at once, and never on the tape, a \
                   match that is not running and a cell off the window or on impassable \
                   ground; the arena, at its next pulse, refuses an occupied cell or a round \
                   in flight to or from the unit, and every place it receives is on the \
                   tape, placed or place_refused, unless the match ends in that pulse. Its \
                   pilot is told nothing; other pilots' sensors find it where it now is.\n\
                   @arena/watch <match> | off         (staff) watch a running match: its \
                   whole field reaches your panel on arena.observer at most once an arena \
                   second, every unit by callsign, cell and object number, and screen 0 of \
                   the TUI and the web draws it; the line client prints none of it. For a \
                   staffer with no centre slot and no seat; at most eight a match. `off` \
                   stops it, and so do a disconnect and the match's end; bare says what you \
                   watch."
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
    // One primary dispatcher: character authoring and the staff-only /live
    // lane share a verb but retain their own authority and persistence checks.
    workshop.register(reg)?;
    // 17g C2c (C2 plan §2.2): the player market's `+minds`, registered
    // wherever `@mind` is and refusing `NOT_OPEN` without a composed market
    // read (`minds::market_commands::market`).
    crate::minds::market_commands::market::register(reg)?;
    // 17g C3a (the C3 plan §2.3): the market's two panel feeds, each answered
    // by its same-named query, registered on every launch format so the
    // manifest is stable; without the command slot or its market read both
    // answer the closed keyframe (`minds::market_commands::query`).
    {
        use crate::minds::market_commands::query::{
            HOLDINGS_FEED, HoldingsPanel, MARKET_FEED, MarketPanel,
        };
        drop(reg.panel_feed(MARKET_FEED)?);
        drop(reg.panel_feed(HOLDINGS_FEED)?);
        reg.query_handler(MARKET_FEED, Arc::new(MarketPanel))?;
        reg.query_handler(HOLDINGS_FEED, Arc::new(HoldingsPanel))?;
    }
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
///
/// **17b cut L.1 — requested per leader** (doc 17 §4, D-UM-53). The request is
/// the squad's lance-leader's, read off the match record's `leaders` beside the
/// roster and carried on the brief ([`crate::deliberation::SquadBrief::leader`])
/// so the order comes back with it as its author. A squad with no leader runs
/// on doctrine: its signal is skipped, again **before** an in-flight slot is
/// taken.
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
        let Some((units, leader)) = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .and_then(|record| {
                record
                    .squads
                    .iter()
                    .find(|roster| roster.id == signal.squad)
                    .map(|roster| {
                        (
                            roster.units.len(),
                            crate::deliberation::leader_of(&record.leaders, signal.squad).cloned(),
                        )
                    })
            })
        else {
            continue;
        };
        // 17b cut L.1 (D-UM-53): no character leads the squad, so nobody asks.
        let Some(leader) = leader else {
            tracing::debug!(
                %match_id, squad = %signal.squad,
                "tacsim: no character leads the squad; it runs on doctrine"
            );
            continue;
        };
        {
            let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
                return;
            };
            let arena_ms = registry.matches.get(&match_id).map(|m| m.now_ms);
            let state = registry.deliberation.entry(match_id).or_default();
            // 16b cut Y.1: a pilot's order stands; nothing is asked over it.
            if state.human_holds(signal.squad, now_ms, arena_ms) {
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
            leader,
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
///
/// **17b cut L.1 — the leader's wall** (doc 17 §4, D-UM-52, D-UM-53). An order
/// whose author ([`crate::deliberation::Order::by`]) is not the match record's
/// lance-leader of that squad is dropped and counted `refused` — after the
/// shield and the roster lookup, so a stale reply for an ended match still
/// returns quietly and creates no state. [`send_order`]'s admission is the
/// second wall and would refuse every unit anyway.
///
/// **And an undecodable reply gives its slot back** (17b cut L.1): one that
/// lacks `by` — or anything else serde refuses — is read once more as plain
/// JSON for its top-level `match_id`, and that match's in-flight slot is
/// released before the reply is dropped. Until this cut such a reply returned
/// before `release_in_flight` and held one of the match's
/// [`crate::deliberation::MAX_IN_FLIGHT_PER_MATCH`] slots for the match's life.
pub fn on_deliberation_reply(ctx: &mut Ctx<'_>, payload: &str) {
    let Ok(reply) = serde_json::from_str::<crate::deliberation::DeliberationReply>(payload) else {
        // Both reply shapes carry `match_id` at the top level, so a payload that
        // is JSON at all names the match whose slot it held. Released once, here;
        // the return skips the decoded path's own release.
        if let Some(match_id) = serde_json::from_str::<serde_json::Value>(payload)
            .ok()
            .and_then(|value| value.get("match_id").and_then(serde_json::Value::as_u64))
        {
            release_in_flight(ctx, MatchId(match_id));
        }
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
                .is_some_and(|registry| {
                    registry.deliberation.get(&match_id).is_some_and(|state| {
                        state.human_holds(
                            order.squad,
                            now_ms,
                            registry.matches.get(&match_id).map(|m| m.now_ms),
                        )
                    })
                })
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
            let Some((units, leader)) = ctx
                .w
                .module_state::<crate::match_::MatchBook>()
                .and_then(|book| book.get(match_id))
                .and_then(|record| {
                    record
                        .squads
                        .iter()
                        .find(|roster| roster.id == order.squad)
                        .map(|roster| {
                            (
                                roster.units.clone(),
                                crate::deliberation::leader_of(&record.leaders, order.squad)
                                    .cloned(),
                            )
                        })
                })
            else {
                return;
            };
            // 17b cut L.1 (D-UM-52, D-UM-53): only the squad's own lance-leader
            // gives it a deliberated order. A leaderless squad, or a reply whose
            // author is anybody else, is dropped and counted.
            if leader.as_ref() != Some(&order.by) {
                if let Some(state) = ctx
                    .w
                    .module_state_mut::<ArenaRegistry>()
                    .and_then(|registry| registry.deliberation.get_mut(&match_id))
                {
                    state.refused = state.refused.saturating_add(1);
                }
                tracing::debug!(
                    %match_id, squad = %order.squad,
                    "arena deliberation reply dropped: its author does not lead the squad"
                );
                return;
            }
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
                        deadline: OrderDeadline::World(expires_ms),
                        objective: objective.clone(),
                        said: false,
                        issuer: None,
                        by: order.by.clone(),
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
/// [`forward_intent`] makes are all about *a player's authority over a tank*.
/// **Since 17b cut L.1** (doc 17 §4, D-UM-52) the authority a squad order needs
/// is two: **a character entitled to order the squad** gave it — `admit_order`,
/// run first, on the cockpit's own `command_of` rule, so the typed path
/// ([`forward_order`]), the leader's path ([`on_deliberation_reply`]) and any
/// direct caller meet one wall — and **the unit is on a machine controller**,
/// which is a fact only the arena holds and which it checks itself
/// ([`crate::arena::intents::Refusal::NotMachine`], `NotReflex` until 17b cut
/// M4.6).
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
    admit_order(ctx.w, match_id, unit, &order)?;
    offer_helm(ctx, match_id, unit, IntentKind::Order(Box::new(order)))
}

/// **The world and bridge admission of one squad order** (17b cut L.1; doc 17
/// §4 "A mind never orders", D-UM-52, D-UM-53): is
/// [`crate::deliberation::Order::by`] a character entitled to order `unit` of
/// `order.squad` in `match_id`?
///
/// **The rule is the cockpit's own** (`crate::match_::cockpit::command_of`, 16c
/// Q38 as 16b cut Y.1 built it) and nothing weaker:
///
/// - `Character { id }`: `id` is at a seat — [`crate::match_::cockpit::flying`]
///   answers a seat whose `driver` is `id`, so the character named is the one at
///   the seat — and `command_of` on that seat answers `match_id` with a grant of
///   `order.squad` for `unit`: the side's lead (`Command::Squads`) orders any
///   roster of the side; anyone else (`Command::Wingman`) only the one machine
///   of their pod pair. A non-lead is **not** admitted for a squad unit that is
///   not their wingman, even on their own side.
/// - `Persona { squad }`: the persona leads that squad (`squad == order.squad`),
///   the order is written in the commander's scope (`issuer` is `None`; D-UM-53:
///   a leader's hint scope is the commander's), the match record's `leaders`
///   name exactly this persona for the squad, and the record's roster of that
///   squad holds `unit` — the same `holds` the lead's arm reads, so a leader
///   orders its own squad's machines and no other's (the reply path fans out to
///   exactly that roster).
///
/// `offer_helm` stays principal-free: a `ReleaseOrder` carries no author, and
/// D-UM-52 is about `ORDER`.
///
/// # Errors
///
/// [`IntentRefusal::Unavailable`] with no registry, no match book or no record;
/// [`IntentRefusal::NoCharacter`] for anything else that fails the rule.
pub(crate) fn admit_order(
    w: &mush_core::World,
    match_id: MatchId,
    unit: ObjId,
    order: &crate::deliberation::Order,
) -> Result<(), IntentRefusal> {
    if crate::detachment::live::owns_unit(w, match_id, unit) {
        return Err(IntentRefusal::NoCharacter);
    }
    if w.module_state::<ArenaRegistry>().is_none() {
        return Err(IntentRefusal::Unavailable);
    }
    let record = w
        .module_state::<crate::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .ok_or(IntentRefusal::Unavailable)?;
    let entitled = match &order.by {
        CharacterRef::Character { id } => {
            crate::match_::cockpit::flying(w, *id).is_some_and(|seat| {
                seat.driver == *id
                    && order.issuer == Some(seat.unit)
                    && crate::match_::cockpit::require_alive(w, *id).is_ok()
                    && command_grants(w, &seat, match_id, order.squad, unit)
            })
        }
        CharacterRef::Persona { squad } => {
            *squad == order.squad
                && order.issuer.is_none()
                && crate::deliberation::leader_of(&record.leaders, order.squad) == Some(&order.by)
                && record
                    .squads
                    .iter()
                    .any(|roster| roster.id == order.squad && roster.holds(unit))
        }
    };
    if entitled {
        Ok(())
    } else {
        Err(IntentRefusal::NoCharacter)
    }
}

/// **Whether `seat`'s command grants `squad`'s `unit` in `match_id`** —
/// [`admit_order`]'s reading of `crate::match_::cockpit::command_of`: a lead's
/// `Squads` holding a roster `squad` that holds `unit` (the cockpit's own `ORDER`
/// fans out to exactly that roster's units), or a `Wingman` naming that squad and
/// that very unit.
fn command_grants(
    w: &mush_core::World,
    seat: &crate::match_::cockpit::Seat,
    match_id: MatchId,
    squad: crate::deliberation::SquadId,
    unit: ObjId,
) -> bool {
    use crate::match_::cockpit::Command;
    match crate::match_::cockpit::command_of(w, seat) {
        (Some(at), _, Command::Squads(rosters)) => {
            at == match_id
                && rosters
                    .iter()
                    .any(|roster| roster.id == squad && roster.holds(unit))
        }
        (
            Some(at),
            _,
            Command::Wingman {
                squad: wing_squad,
                unit: wing,
                ..
            },
        ) => at == match_id && wing_squad == squad && wing == unit,
        _ => false,
    }
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
    if matches!(kind, IntentKind::Order(_) | IntentKind::ReleaseOrder { .. })
        && crate::detachment::live::owns_unit(ctx.w, match_id, unit)
    {
        return Err(IntentRefusal::NoCharacter);
    }
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

/// Offer the current host fences before controller commands. A full queue keeps
/// the pending message for the next tick. Queue delivery never claims execution.
pub(crate) fn flush_detachment_dispatches(ctx: &mut Ctx<'_>) {
    use crate::arena::detachment::{
        DetachmentAuthority, DetachmentCommand, DetachmentControl, DetachmentObjective,
    };
    use crate::detachment::live::{self, LiveDispatch};
    use mush_tactics::detachment::Directive;

    // Confirming an authority offer exposes its command on the second pass.
    for _ in 0..2 {
        for event in live::drain_dispatches(ctx.w) {
            match event {
                LiveDispatch::Authority {
                    match_id,
                    unit,
                    epoch,
                    detachment,
                    revision,
                    term,
                } => {
                    let Some(registry) = ctx.w.module_state::<ArenaRegistry>() else {
                        return;
                    };
                    if registry.match_of_unit(unit) != Some(match_id) {
                        continue;
                    }
                    let control = DetachmentControl {
                        unit,
                        epoch,
                        authority: Some(DetachmentAuthority {
                            detachment: detachment.0,
                            revision,
                            term,
                        }),
                    };
                    if registry
                        .tasks
                        .offer(match_id, ArenaMsg::DetachmentControl(control))
                        .is_ok()
                    {
                        live::confirm_authority(ctx.w, match_id, unit, epoch);
                    }
                }
                LiveDispatch::ControllerOrder {
                    match_id,
                    authority_epoch,
                    squad,
                    order,
                    expires_after_ms,
                } => {
                    let Some(now_ms) = ctx
                        .w
                        .module_state::<ArenaRegistry>()
                        .and_then(|registry| registry.mirror(match_id))
                        .map(|mirror| mirror.now_ms)
                    else {
                        continue;
                    };
                    let expires_at_ms = expires_after_ms.map(|remaining| {
                        now_ms.saturating_add(i64::try_from(remaining).unwrap_or(i64::MAX))
                    });
                    let directive = match &order.directive {
                        Directive::Cancel => None,
                        Directive::Objective(payload) => Some(DetachmentObjective {
                            objective: payload.objective(),
                            constraints: payload.constraints(),
                        }),
                    };
                    let command = DetachmentCommand {
                        authority: DetachmentAuthority {
                            detachment: order.detachment.0,
                            revision: order.roster_revision,
                            term: order.command_term,
                        },
                        authority_epoch,
                        sequence: order.sequence,
                        issuer: order.issuer,
                        issuer_unit: order.issuer_unit,
                        squad,
                        directive,
                        expires_at_ms,
                    };
                    if offer_helm(
                        ctx,
                        match_id,
                        order.recipient.unit,
                        IntentKind::DetachmentCommand(Box::new(command)),
                    )
                    .is_ok()
                        && let Err(error) = live::confirm_delivery(
                            ctx,
                            order.detachment,
                            order.recipient.slot,
                            order.sequence,
                        )
                    {
                        tracing::warn!(%error, "detachment delivery confirmation refused");
                    }
                }
            }
        }
    }
}

/// **The lapse sentence** (17b cut L.2; D-UM-32 as amended by R-C, D-UM-53): *the
/// leader's orders lapsing*, in the leader's name — `Squad red-1: red lance leader
/// 1's hold order has run out; back on their minds.`, or `your` to the pilot who
/// gave it.
///
/// The clause names the order's author and claims nothing about "the squad": a
/// non-lead's `ORDER` reaches one wingman while the watch is per squad, so the
/// `Squad {squad}:` prefix is the watch's key and `back on their minds` is true
/// of whatever the order reached. It no longer says the link dropped: that is
/// the words of a mind's own failure (doc 17 §8, M3's `MindDropped`), so the two
/// vocabularies do not collide.
///
/// **17b cut M5.6** (the M5.3 close's D9 ruling): `back on doctrine.` until that
/// cut. A lapsed order leaves each hull to the mind sealed for it, which a
/// lead's `SIM AI MIND` may make another than Standard Doctrine (a fixture's
/// before 17b cut M6), so the sentence names none — one word for every mind,
/// as the release acknowledgement (`Released red-1 to its mind`) says it.
fn lapse_line(squad: crate::deliberation::SquadId, whose: &str, objective: &str) -> String {
    format!("Squad {squad}: {whose} {objective} order has run out; back on their minds.")
}

/// **The one in-world notice doc 16 §6 asks for**, said once per squad whose order
/// ran out unrenewed, in [`lapse_line`]'s words and in the name of the order's
/// author (`OrderWatch::by`, 17b cut L.2).
///
/// Who hears it depends on who gave it:
///
/// - **a lance-leader** (`CharacterRef::Persona`): every **pilot of the squad's own
///   side** in that match, because their AI wingmen just went back on their own
///   minds — *`Squad red-1: red lance leader 1's hold order has run out; back on
///   their minds.`*;
/// - **a character** (`CharacterRef::Character`, a seated pilot's `ORDER`): that
///   character **alone**, as *`Squad red-1: your hold order has run out; back on
///   their minds.`* (each `back on doctrine.` until 17b cut M5.6; re-quoted by 17b
///   cut M5.7) — 16c Q38's "no radio line" never told the side a pilot ordered,
///   so its lapse is not told to them either (a declared narrowing of the sentence
///   this replaced, which reached the whole side);
///
/// and every connected wizard either way, because staff are who watch a match, on
/// one line naming the author (the character's current name, `its leader's` once
/// the object is gone). The squad's own tanks hear nothing — they have no pilots,
/// which is why they have a leader.
fn say_expired_orders(ctx: &mut Ctx<'_>) {
    say_due_orders(ctx, None);
}

// Finish says only recovered watches due at the actual terminal arena clock.
// Ordinary world watch behavior at finish remains unchanged.
fn say_due_orders(ctx: &mut Ctx<'_>, terminal: Option<MatchId>) {
    let now_ms = ctx.w.now().saturating_mul(1_000);
    let mut due: Vec<(MatchId, crate::deliberation::SquadId, SmolStr, CharacterRef)> = Vec::new();
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        for (match_id, state) in &mut registry.deliberation {
            if terminal.is_some_and(|id| id != *match_id) {
                continue;
            }
            let arena_ms = registry.matches.get(match_id).map(|m| m.now_ms);
            for (squad, watch) in &mut state.orders {
                if terminal.is_some() && !matches!(watch.deadline, OrderDeadline::Arena(_)) {
                    continue;
                }
                if !watch.said && watch.deadline.due(now_ms, arena_ms) {
                    watch.said = true;
                    due.push((*match_id, *squad, watch.objective.clone(), watch.by.clone()));
                }
            }
        }
    }
    for (match_id, squad, objective, by) in due {
        let staff_line = match by {
            CharacterRef::Persona { squad: leads } => {
                let whose = format!("{}'s", CharacterRef::persona_name(leads));
                let line = lapse_line(squad, &whose, &objective);
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
                line
            }
            CharacterRef::Character { id } => {
                let name = ctx.w.get(id).map(|object| object.name.to_string());
                if name.is_some() {
                    ctx.out.sys_to_actor(
                        id,
                        NOTICE_CATEGORY,
                        lapse_line(squad, "your", &objective),
                    );
                }
                let whose =
                    name.map_or_else(|| "its leader's".to_owned(), |name| format!("{name}'s"));
                lapse_line(squad, &whose, &objective)
            }
        };
        notify_staff(ctx, format!("Arena match {match_id}: {staff_line}"));
    }
}

/// The cause word of a think that ran out of its op budget (`mush_mind`'s
/// `Fault::Ops`), the one strikes shape whose sentence names the budget.
const OPS_FAULT: &str = "ops";

/// Every tripped sentence's second half (17b cut M3.4; D-UM-32).
const ROM_TAIL: &str = "The hull is on the ROM; MIND is off for the match.";

/// **The tripped sentence** (17b cut M3.4; doc 17 §8, D-UM-32): what the
/// warrant principal of a hull dropped to the ROM reads, in the words of the
/// tape's `mind_dropped` line — its cause, its last fault's word and the op
/// budget the faulting thinks ran under, never a config read (16b cut V.3's
/// rule: a reload cannot change how a match is told). `whose` is `Your` where
/// the principal is the hull's own seated pilot — a pilot's released hull,
/// flown by a mind sealed in that pilot's name (since 17b cut M5.2 the mind
/// the pilot chose) — and `{callsign}'s` otherwise (`RB's`: the fill, told to
/// its side's lead):
///
/// * strikes, the last fault `ops`: `Your core tripped: over its 1,500-op
///   budget three thinks running. The hull is on the ROM; MIND is off for the
///   match.`;
/// * strikes, any other last fault: `Your core tripped: three faults running,
///   the last {word}. The hull is on the ROM; MIND is off for the match.`
///   (`unknown` where the line carries no word, which the arena never
///   writes);
/// * core: `Your core is destroyed. The hull is on the ROM; MIND is off for
///   the match.`;
/// * abi: `Your core cannot run its mind on this engine. The hull is on the
///   ROM; MIND is off for the match.` (written by nothing in the arena: a
///   resume across a changed engine is refused by name).
///
/// At most 117 columns at the largest class budget and a two-letter callsign
/// (pinned in this file's tests).
#[must_use]
pub fn dropped_line(
    whose: &str,
    cause: crate::arena::mind::DropCause,
    fault: Option<&str>,
    ops: Option<u64>,
) -> String {
    use crate::arena::mind::DropCause;
    let what = match cause {
        DropCause::Strikes => match (fault, ops) {
            (Some(OPS_FAULT), Some(ops)) => format!(
                "{whose} core tripped: over its {}-op budget three thinks running.",
                thousands(ops)
            ),
            (fault, _) => format!(
                "{whose} core tripped: three faults running, the last {}.",
                fault.unwrap_or("unknown")
            ),
        },
        DropCause::Core => format!("{whose} core is destroyed."),
        DropCause::Abi => format!("{whose} core cannot run its mind on this engine."),
    };
    format!("{what} {ROM_TAIL}")
}

/// **The staff form of the tripped sentence** (17b cut M3.4; D-UM-32): the
/// hull by its callsign and the cause in the same words, shortened under the
/// 120-column floor — `{callsign} dropped to the ROM: over its 1,500-op
/// budget three thinks running.` / `…: three faults running, the last
/// {word}.` / `…: its core is destroyed.` / `…: its core cannot run its mind
/// on this engine.` — which the bridge's staff fan-out (`notify_staff`) is
/// handed behind `Arena match {id}: `, as the lapse line is.
#[must_use]
pub fn staff_line(
    callsign: &str,
    cause: crate::arena::mind::DropCause,
    fault: Option<&str>,
    ops: Option<u64>,
) -> String {
    use crate::arena::mind::DropCause;
    let why = match cause {
        DropCause::Strikes => match (fault, ops) {
            (Some(OPS_FAULT), Some(ops)) => {
                format!("over its {}-op budget three thinks running", thousands(ops))
            }
            (fault, _) => format!(
                "three faults running, the last {}",
                fault.unwrap_or("unknown")
            ),
        },
        DropCause::Core => "its core is destroyed".to_owned(),
        DropCause::Abi => "its core cannot run its mind on this engine".to_owned(),
    };
    format!("{callsign} dropped to the ROM: {why}.")
}

/// **The one line a drop to the ROM is told in** (17b cut M3.4; doc 17 §8,
/// D-UM-32), for every `mind_dropped` line of one chunk, in line order:
///
/// - **the warrant principal alone** — `MatchRecord::minds[unit]`'s seal's
///   `principal` (17b cut M3.3): a `CharacterRef::Character` whose object
///   still exists is told [`dropped_line`] (`Your` when it is the hull's own
///   seated pilot, `{callsign}'s` otherwise — the side lead, for the fill); a
///   lance-leader persona, or nobody (a staff `@arena/start` match, a side
///   with no lead), is told nothing;
/// - **staff**, always, through [`notify_staff`] with [`staff_line`].
///
/// Nobody else: the other side reads nothing, and no line names an object
/// number (the observer rule — the principal's own side's hull, by its
/// callsign). The words are the LINE's: its cause, its `fault` and its `ops`,
/// never a config read. A line whose cause word is none of the three, or a
/// match the book no longer holds, is told to nobody. Per chunk, never per
/// pulse.
fn say_mind_dropped(ctx: &mut Ctx<'_>, match_id: MatchId, events: &[LedgerEvent]) {
    use crate::arena::mind::DropCause;
    let drops: Vec<(ObjId, DropCause, Option<SmolStr>, Option<u64>)> = events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::MindDropped {
                unit,
                cause,
                fault,
                ops,
            } => Some((*unit, DropCause::parse(cause)?, fault.clone(), *ops)),
            _ => None,
        })
        .collect();
    for (unit, cause, fault, ops) in drops {
        let Some((callsign, principal, own)) = ctx
            .w
            .module_state::<crate::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .map(|record| {
                let principal = record
                    .minds
                    .get(&unit)
                    .and_then(|sealed| sealed.seal.principal.clone());
                let own = matches!(
                    &principal,
                    Some(CharacterRef::Character { id })
                        if record
                            .teams
                            .iter()
                            .any(|roster| roster.pilots.contains(&(*id, unit)))
                );
                (record.callsign_word(unit), principal, own)
            })
        else {
            continue;
        };
        if let Some(CharacterRef::Character { id }) = principal
            && ctx.w.get(id).is_some()
        {
            let whose = if own {
                "Your".to_owned()
            } else {
                format!("{callsign}'s")
            };
            ctx.out.sys_to_actor(
                id,
                NOTICE_CATEGORY,
                dropped_line(&whose, cause, fault.as_deref(), ops),
            );
        }
        notify_staff(
            ctx,
            format!(
                "Arena match {match_id}: {}",
                staff_line(&callsign, cause, fault.as_deref(), ops)
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    #[path = "leader_rows.rs"]
    mod leader_rows;

    #[path = "resume_watch.rs"]
    mod resume_watch;

    use mush_core::{GOD, ObjId};
    use smol_str::SmolStr;

    use super::{
        ArenaRegistry, dropped_line, lapse_line, open_match, sources_by_seal, staff_line, thousands,
    };
    use crate::arena::mind::DropCause;
    use crate::arena::state::{PartyId, Pilot};
    use crate::arena::test_support::{custody, pose, tank};
    use crate::arena_task::MatchId;
    use crate::deliberation::{CharacterRef, SquadId};

    /// **17b cut L.2**: the lapse sentence's two shapes, byte for byte — the
    /// issuing pilot's own (`your`) and a lance-leader's in its persona's name —
    /// with no "link dropped" left in either (D-UM-32 as amended by R-C).
    ///
    /// Both ended `back on doctrine.` until 17b cut M5.6 (D9): `back on their
    /// minds.`, each under 120 columns at the widest squad, persona and
    /// objective the arena words.
    #[test]
    fn l2_the_lapse_line_has_two_shapes() {
        let red_one = SquadId {
            team: PartyId::RED,
            index: 1,
        };
        assert_eq!(
            lapse_line(red_one, "your", "hold"),
            "Squad red-1: your hold order has run out; back on their minds."
        );
        let whose = format!("{}'s", CharacterRef::persona_name(red_one));
        let persona = lapse_line(red_one, &whose, "hold");
        assert_eq!(
            persona,
            "Squad red-1: red lance leader 1's hold order has run out; back on their minds."
        );
        assert!(!persona.contains("link dropped"));
        assert!(!persona.contains("doctrine"));
        let widest = SquadId {
            team: PartyId::BLUE,
            index: 6,
        };
        let whose = format!("{}'s", CharacterRef::persona_name(widest));
        for objective in ["hold", "advance", "engage", "withdraw", "screen", "support"] {
            let line = lapse_line(widest, &whose, objective);
            assert!(line.chars().count() < 120, "{line}");
        }
    }

    // -----------------------------------------------------------------------
    // 17b cut M3.4 — the tripped sentence, worded from the tape
    // -----------------------------------------------------------------------

    /// One player shape: `(whose, cause, fault, ops, sentence)`.
    type Shape = (
        &'static str,
        DropCause,
        Option<&'static str>,
        Option<u64>,
        &'static str,
    );

    /// The eight player shapes.
    fn the_eight() -> Vec<Shape> {
        vec![
            (
                "Your",
                DropCause::Strikes,
                Some("ops"),
                Some(1_500),
                "Your core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "Your",
                DropCause::Strikes,
                Some("limit"),
                Some(1_500),
                "Your core tripped: three faults running, the last limit. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "Your",
                DropCause::Core,
                None,
                None,
                "Your core is destroyed. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "Your",
                DropCause::Abi,
                None,
                None,
                "Your core cannot run its mind on this engine. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "RB's",
                DropCause::Strikes,
                Some("ops"),
                Some(1_500),
                "RB's core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "RB's",
                DropCause::Strikes,
                Some("limit"),
                Some(1_500),
                "RB's core tripped: three faults running, the last limit. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "RB's",
                DropCause::Core,
                None,
                None,
                "RB's core is destroyed. The hull is on the ROM; MIND is off for the match.",
            ),
            (
                "RB's",
                DropCause::Abi,
                None,
                None,
                "RB's core cannot run its mind on this engine. The hull is on the ROM; MIND is off for the match.",
            ),
        ]
    }

    /// **17b cut M3.4: the tripped sentences, byte for byte, with their
    /// lengths** (doc 17 §8, D-UM-32): the eight player shapes — `Your` and
    /// the fill's `RB's`, each on an `ops` strike (the budget, with a
    /// thousands separator), any other strike (the last word), a destroyed
    /// core and an engine that cannot run the mind — and the four staff
    /// lines behind `Arena match {id}: ` with a five-digit id. Every player
    /// line at most 117 columns (the longest is the Class III budget's,
    /// `18,000`); every staff line under 100. Every other last word (`limit`,
    /// `type`, `memory`, `intents`, `unknown`, `panic`) takes the second
    /// shape; a strike line with no word reads `unknown`.
    #[test]
    fn m3_the_tripped_sentences_are_pinned_byte_for_byte_with_their_lengths() {
        let lengths = [116_usize, 107, 74, 96, 116, 107, 74, 96];
        for ((whose, cause, fault, ops, sentence), length) in the_eight().into_iter().zip(lengths) {
            assert_eq!(dropped_line(whose, cause, fault, ops), sentence);
            assert_eq!(sentence.chars().count(), length, "{sentence}");
        }
        let longest = dropped_line("RB's", DropCause::Strikes, Some("ops"), Some(18_000));
        assert_eq!(
            longest,
            "RB's core tripped: over its 18,000-op budget three thinks running. The hull is on the ROM; MIND is off for the match."
        );
        assert_eq!(longest.chars().count(), 117, "the longest player line");
        for word in ["limit", "type", "memory", "intents", "unknown", "panic"] {
            let line = dropped_line("RB's", DropCause::Strikes, Some(word), Some(1_500));
            assert_eq!(
                line,
                format!(
                    "RB's core tripped: three faults running, the last {word}. The hull is on the ROM; MIND is off for the match."
                )
            );
            assert!(line.chars().count() <= 117, "{line}");
        }
        assert_eq!(
            dropped_line("RB's", DropCause::Strikes, None, Some(1_500)),
            "RB's core tripped: three faults running, the last unknown. The hull is on the ROM; MIND is off for the match."
        );
        // The thousands separator.
        for (n, written) in [
            (0_u64, "0"),
            (400, "400"),
            (1_500, "1,500"),
            (6_000, "6,000"),
            (18_000, "18,000"),
            (1_234_567, "1,234,567"),
        ] {
            assert_eq!(thousands(n), written);
        }
        // The four staff lines, with a five-digit match id.
        let id = MatchId(12_345);
        let staff = [
            (
                staff_line("RB", DropCause::Strikes, Some("ops"), Some(1_500)),
                "Arena match 12345: RB dropped to the ROM: over its 1,500-op budget three thinks running.",
                88_usize,
            ),
            (
                staff_line("RB", DropCause::Strikes, Some("limit"), Some(1_500)),
                "Arena match 12345: RB dropped to the ROM: three faults running, the last limit.",
                79,
            ),
            (
                staff_line("RB", DropCause::Core, None, None),
                "Arena match 12345: RB dropped to the ROM: its core is destroyed.",
                64,
            ),
            (
                staff_line("RB", DropCause::Abi, None, None),
                "Arena match 12345: RB dropped to the ROM: its core cannot run its mind on this engine.",
                86,
            ),
        ];
        for (line, want, length) in staff {
            let line = format!("Arena match {id}: {line}");
            assert_eq!(line, want);
            assert_eq!(line.chars().count(), length, "{line}");
            assert!(length < 100);
        }
        let widest = format!(
            "Arena match {id}: {}",
            staff_line("RB", DropCause::Strikes, Some("ops"), Some(18_000))
        );
        assert_eq!(widest.chars().count(), 89);
        assert_eq!(
            DropCause::parse("strikes"),
            Some(DropCause::Strikes),
            "the tape's word reads back"
        );
        assert_eq!(DropCause::parse("core"), Some(DropCause::Core));
        assert_eq!(DropCause::parse("abi"), Some(DropCause::Abi));
        assert_eq!(DropCause::parse("ROM"), None);
    }

    /// Red's pilot, red's fill, blue's pilot, blue's fill, and a staff-run
    /// hull nobody leads.
    const RA: ObjId = ObjId(9_301);
    const RB: ObjId = ObjId(9_302);
    const BA: ObjId = ObjId(9_401);
    const BB: ObjId = ObjId(9_402);
    const XA: ObjId = ObjId(9_403);
    const DROP_MATCH: MatchId = MatchId(81);

    /// A running record of `DROP_MATCH`: red's `RA` piloted by `ada` and its
    /// fill `RB` led by her; blue's `BA` piloted by `bo`, its fill `BB` under
    /// a lance-leader persona and `BC` (`XA`) sealed with no principal.
    fn dropped_record(ada: ObjId, bo: ObjId) -> crate::match_::MatchRecord {
        use crate::arena::state::Designator;
        let slot = |team: PartyId, slot: u16| Designator::Unit {
            party: team,
            letter: char::from(
                b'A'.saturating_add(u8::try_from(slot).unwrap_or(1).saturating_sub(1)),
            ),
        };
        let sealed = |unit: ObjId, principal: Option<CharacterRef>| {
            (
                unit,
                crate::match_::MindSealRecord {
                    sortie: crate::arena::mind::SortieId::in_match(DROP_MATCH.0, unit),
                    seal: crate::arena::mind::MindSeal {
                        unit,
                        principal,
                        ..crate::arena::mind::MindSeal::placeholder()
                    },
                },
            )
        };
        crate::match_::MatchRecord {
            id: DROP_MATCH,
            state: crate::match_::MatchState::Running,
            arena: SmolStr::new("tacsim.arena.proving_ground"),
            arena_definition_digest: "ab".to_owned(),
            teams: vec![
                crate::match_::TeamRoster {
                    team: PartyId::RED,
                    units: vec![RA, RB],
                    pilots: vec![(ada, RA)],
                },
                crate::match_::TeamRoster {
                    team: PartyId::BLUE,
                    units: vec![BA, BB, XA],
                    pilots: vec![(bo, BA)],
                },
            ],
            started_ms: 1_000,
            ended: None,
            ledger_log_id: format!("match.{}", DROP_MATCH.0),
            ledger_digest: String::new(),
            seed: 2_350,
            last_pose: std::collections::BTreeMap::new(),
            squads: Vec::new(),
            leaders: Vec::new(),
            callsigns: std::collections::BTreeMap::from([
                (RA, slot(PartyId::RED, 1)),
                (RB, slot(PartyId::RED, 2)),
                (BA, slot(PartyId::BLUE, 1)),
                (BB, slot(PartyId::BLUE, 2)),
                (XA, slot(PartyId::BLUE, 3)),
            ]),
            folds: std::collections::BTreeMap::new(),
            unparsed: 0,
            replay: None,
            mode: crate::arena::MatchMode::Skirmish,
            minds: std::collections::BTreeMap::from([
                sealed(RA, Some(CharacterRef::Character { id: ada })),
                sealed(RB, Some(CharacterRef::Character { id: ada })),
                sealed(BA, Some(CharacterRef::Character { id: bo })),
                sealed(
                    BB,
                    Some(CharacterRef::Persona {
                        squad: SquadId {
                            team: PartyId::BLUE,
                            index: 1,
                        },
                    }),
                ),
                sealed(XA, None),
            ]),
        }
    }

    /// One `mind_dropped` line.
    fn drop_line(
        seq: u64,
        unit: ObjId,
        cause: &str,
        fault: Option<&str>,
        ops: Option<u64>,
    ) -> crate::arena::ledger::LedgerEvent {
        crate::arena::ledger::LedgerEvent {
            seq,
            t_ms: 4_945,
            kind: crate::arena::ledger::EventKind::MindDropped {
                unit,
                cause: SmolStr::new(cause),
                fault: fault.map(SmolStr::new),
                ops,
            },
            draws: Vec::new(),
            hash: 7,
        }
    }

    /// Every `(audience, text)` an outbox holds.
    fn told(out: &mush_core::Outbox) -> Vec<(ObjId, String)> {
        out.events()
            .iter()
            .filter_map(|(audience, event)| match audience {
                mush_core::Audience::Actor(actor) => Some((*actor, mush_testkit::text_of(event))),
                _ => None,
            })
            .collect()
    }

    /// A world with the tacsim layer on `cfg`, `ada` and `bo` spawned, staff
    /// connected, and [`dropped_record`] tracked.
    fn dropped_world(cfg: crate::TacsimConfig) -> (mush_testkit::TestWorld, ObjId, ObjId) {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(cfg),
        )]);
        let ada = tw.spawn_player("Ada");
        let bo = tw.spawn_player("Bo");
        let _ = tw.connect(GOD);
        let _ = tw.connect(ada);
        let _ = tw.connect(bo);
        if let Some(book) = tw
            .world_mut()
            .module_state_mut::<crate::match_::MatchBook>()
        {
            book.insert_for_tests(dropped_record(ada, bo));
        }
        (tw, ada, bo)
    }

    #[test]
    fn m6_trace_world_reads_recheck_connection_principal_and_current_fill_setter() {
        use crate::arena::author_trace::TraceStore;
        use mush_core::{AccountId, AccountIdentity};
        struct Accounts(Vec<(ObjId, AccountId)>);
        impl AccountIdentity for Accounts {
            fn account_of(&self, actor: ObjId) -> Option<AccountId> {
                self.0
                    .iter()
                    .find(|(id, _)| *id == actor)
                    .map(|(_, account)| *account)
            }
        }
        let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
        let ada_account = AccountId::mint("ada", 1).expect("account");
        let bo_account = AccountId::mint("bo", 1).expect("account");
        tw.world_mut()
            .module_state_insert(mush_sim::AccountOracle::new(std::sync::Arc::new(Accounts(
                vec![(ada, ada_account), (bo, bo_account)],
            ))));
        let mut traces = TraceStore::default();
        for unit in [RA, RB, BA, XA] {
            traces.record(unit, 0, 0, false, None, &[], &[], None);
        }
        {
            let registry = tw
                .world_mut()
                .module_state_mut::<ArenaRegistry>()
                .expect("registry");
            registry.track(DROP_MATCH);
            for unit in [RA, RB, BA, XA] {
                registry.bind_unit(unit, DROP_MATCH);
            }
            let mirror = registry.matches.get_mut(&DROP_MATCH).expect("mirror");
            for update in traces.take_updates(0) {
                mirror.traces.apply(update);
            }
        }
        for actor in [ada, bo, GOD] {
            tw.world_mut().set_connected(actor, true);
            tw.world_mut().set_live_conns(actor, 1);
        }
        assert!(super::mind_traces(tw.world(), ada, RB, None).is_some());
        assert!(super::mind_traces(tw.world(), bo, RB, None).is_none());
        assert!(super::mind_traces(tw.world(), GOD, RB, None).is_some());
        assert!(super::mind_traces(tw.world(), ada, RB, Some(65)).is_none());
        tw.world_mut().set_live_conns(ada, 0);
        assert!(
            super::mind_traces(tw.world(), ada, RB, None).is_none(),
            "linkdead is not deliverable"
        );
        tw.world_mut().set_live_conns(ada, 1);
        assert!(super::mind_traces(tw.world(), ada, RB, None).is_some());
        {
            let centre = tw
                .world_mut()
                .module_state_mut::<crate::match_::MatchBook>()
                .expect("book")
                .centre_mut();
            centre.match_id = Some(DROP_MATCH);
            centre.leads = vec![(PartyId::BLUE, ada_account)];
            centre.fill_minds = vec![crate::match_::lobby::FillChoice {
                team: PartyId::BLUE,
                by: crate::match_::lobby::FillSetter::Lead(ada_account),
                choice: serde_json::from_value(
                    serde_json::json!({"mind":"Standard Doctrine","profiles":[]}),
                )
                .expect("choice"),
            }];
        }
        assert!(super::mind_traces(tw.world(), ada, XA, None).is_some());
        assert!(super::mind_traces(tw.world(), bo, XA, None).is_none());
        tw.world_mut()
            .module_state_mut::<crate::match_::MatchBook>()
            .expect("book")
            .centre_mut()
            .leads = vec![(PartyId::BLUE, bo_account)];
        assert!(
            super::mind_traces(tw.world(), ada, XA, None).is_none(),
            "old fill setter lost leadership"
        );
        assert!(
            super::mind_traces(tw.world(), bo, XA, None).is_none(),
            "new lead did not set this choice"
        );
        assert!(super::mind_traces(tw.world(), GOD, XA, None).is_some());
    }

    /// **17b cut M3.4: the warrant principal alone is told, and staff**
    /// (doc 17 §8, D-UM-32; the observer rule): over one hand-built chunk
    /// of five `mind_dropped` lines —
    ///
    /// * the fill `RB`'s strikes, led by Ada: Ada reads `RB's core tripped:
    ///   over its 1,500-op budget …` and nobody else of either side;
    /// * Ada's own `RA` (the principal is its own seated pilot: `Your`, the
    ///   shape a pilot's released hull reaches — since 17b cut M5.2 flying
    ///   the mind its pilot chose) on a destroyed core;
    /// * blue's `BB` under a lance-leader persona and `XA` with no principal:
    ///   told to nobody — Bo, blue's own pilot, reads neither;
    ///
    /// and staff read one line each, `Arena match 81: … dropped to the ROM:
    /// …`. No line names an object number. Then the same chunk through
    /// `on_chunk`, on a world whose class table puts Class I at 2,000 ops: the
    /// sentence still says the LINE's 1,500 — the figures are the tape's,
    /// never the config's.
    #[test]
    fn m3_say_mind_dropped_tells_the_principal_alone_and_staff() {
        let events = vec![
            drop_line(1, RB, "strikes", Some("ops"), Some(1_500)),
            drop_line(2, RA, "core", None, None),
            drop_line(3, BB, "strikes", Some("limit"), Some(1_500)),
            drop_line(4, XA, "abi", None, None),
            // A word that is none of the three is told to nobody.
            drop_line(5, BA, "rom", None, None),
        ];
        let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig::default());
        let mut out = mush_core::Outbox::new();
        {
            let engine = tw.engine_mut();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            super::say_mind_dropped(&mut ctx, DROP_MATCH, &events);
        }
        let heard = told(&out);
        let of = |who: ObjId| -> Vec<String> {
            heard
                .iter()
                .filter(|(actor, _)| *actor == who)
                .map(|(_, text)| text.clone())
                .collect()
        };
        assert_eq!(
            of(ada),
            vec![
                "RB's core tripped: over its 1,500-op budget three thinks running. The hull is on the ROM; MIND is off for the match.".to_owned(),
                "Your core is destroyed. The hull is on the ROM; MIND is off for the match.".to_owned(),
            ],
            "the lead reads her fill's line and her own hull's"
        );
        assert!(
            of(bo).is_empty(),
            "blue's pilot reads nothing: {:?}",
            of(bo)
        );
        // Staff read a warning notice (`text_of` renders its frame).
        assert_eq!(
            of(GOD),
            [
                "Arena match 81: RB dropped to the ROM: over its 1,500-op budget three thinks running.",
                "Arena match 81: RA dropped to the ROM: its core is destroyed.",
                "Arena match 81: BB dropped to the ROM: three faults running, the last limit.",
                "Arena match 81: BC dropped to the ROM: its core cannot run its mind on this engine.",
            ]
            .map(|line| format!("[notice arena Warn {line}]"))
            .to_vec(),
            "staff read every line"
        );
        for (_, text) in &heard {
            for unit in [RA, RB, BA, BB, XA] {
                assert!(
                    !text.contains(&unit.0.to_string()),
                    "an object number in {text:?}"
                );
            }
        }

        // Through `on_chunk`, on a class table the line does not match.
        let classes: crate::arena::core::ClassTable =
            serde_json::from_value(serde_json::json!({"I": {"ops": 2_000}}))
                .expect("a class table");
        let (mut tw, ada, bo) = dropped_world(crate::TacsimConfig {
            minds: crate::config::MindsConfig {
                classes,
                ..crate::config::MindsConfig::default()
            },
            ..crate::TacsimConfig::default()
        });
        assert_eq!(
            crate::match_::lobby::minds_config(tw.world())
                .map(|minds| minds.classes.row(crate::arena::core::CoreClass::I).ops),
            Some(2_000),
            "the world's config is not the line's"
        );
        let mut bytes = Vec::new();
        for event in &events {
            bytes.extend_from_slice(
                serde_json::to_string(event)
                    .expect("a line encodes")
                    .as_bytes(),
            );
            bytes.push(b'\n');
        }
        let mut out = mush_core::Outbox::new();
        {
            let engine = tw.engine_mut();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            super::on_chunk(&mut ctx, DROP_MATCH, 1, &bytes);
        }
        let heard = told(&out);
        let ada_heard: Vec<&String> = heard
            .iter()
            .filter(|(actor, _)| *actor == ada)
            .map(|(_, text)| text)
            .collect();
        assert_eq!(ada_heard.len(), 2, "{heard:?}");
        assert!(
            ada_heard.first().is_some_and(
                |line| line.contains("over its 1,500-op budget") && !line.contains("2,000")
            ),
            "the line's figure, never the config's: {ada_heard:?}"
        );
        assert!(heard.iter().all(|(actor, _)| *actor != bo));
        assert_eq!(
            heard.iter().filter(|(actor, _)| *actor == GOD).count(),
            4,
            "{heard:?}"
        );
    }

    /// **16b cut I.4**: `open_match` keeps ONE defensive unbind per pilot it
    /// binds. Unreachable through the verbs — `@arena/watch` refuses a slot
    /// holder, and a slot is claimed only while the centre is not busy — so it
    /// is proved here, on the registry, by binding a watcher by hand and
    /// opening a match that seats them.
    #[tokio::test]
    async fn i4_open_match_unbinds_a_watcher_it_seats_as_a_pilot() {
        const UNIT: ObjId = ObjId(9_101);
        const MATCH: MatchId = MatchId(77);
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::default())]);
        let pilot = tw.spawn_player("Watcher");
        if let Some(registry) = tw.world_mut().module_state_mut::<ArenaRegistry>() {
            registry.bind_watcher(pilot, MatchId(76));
            registry.bind_watcher(GOD, MatchId(76));
        }
        let cfg = crate::config::ArenaConfig::default();
        let mut out = mush_core::Outbox::new();
        let engine = tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(SmolStr::new(crate::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let opened = open_match(
            &mut ctx,
            MATCH,
            custody(
                vec![tank(UNIT, PartyId::RED, pose(1_500, 1_500))],
                vec![Pilot {
                    unit: UNIT,
                    pilot,
                    station: SmolStr::new("cockpit"),
                    epoch: 1,
                }],
            ),
            &cfg,
            &crate::config::MatchConfig::default(),
            &crate::config::SensorsConfig::default(),
        );
        assert!(opened.is_ok(), "the match opens: {opened:?}");
        let registry = ctx
            .w
            .module_state::<ArenaRegistry>()
            .expect("the registry is composed");
        assert_eq!(registry.watching(pilot), None, "the pilot watches nothing");
        assert_eq!(registry.seat_of_pilot(pilot), Some((MATCH, UNIT)));
        assert_eq!(
            registry.watching(GOD),
            Some(MatchId(76)),
            "a watcher who is not seated is left alone"
        );
        let tasks = registry.tasks().clone();
        if let Some((handle, join)) = tasks.remove(MATCH) {
            handle.signal_stop();
            join.abort();
        }
    }

    /// **17b cut M1b.5** (doc 17 §11: a section that drives a station the hull
    /// lacks is shown inert with its reason): `open_match` writes the mirror's
    /// armed set from the check-out — a mind hull with a gun is in it, one with
    /// none is not — and `@mind/live`'s read-back reads it, so the unarmed
    /// hull's fire discipline is inert and the armed one's live, both on Line
    /// `(as checked out)`. The verb's help is three entries, one line each.
    #[tokio::test]
    async fn m1b5_open_match_writes_the_armed_set_the_read_back_reads() {
        use crate::arena::state::Controller;
        use crate::arena::test_support::line_tank;
        const ARMED: ObjId = ObjId(9_201);
        const UNARMED: ObjId = ObjId(9_202);
        const MATCH: MatchId = MatchId(78);
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::default())]);
        let cfg = crate::config::ArenaConfig::default();
        {
            let mut out = mush_core::Outbox::new();
            let engine = tw.engine_mut();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(SmolStr::new(crate::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let opened = open_match(
                &mut ctx,
                MATCH,
                custody(
                    vec![
                        crate::arena::state::CustodyUnit {
                            controller: Controller::Mind,
                            ..line_tank(ARMED, PartyId::RED, pose(1_500, 1_500))
                        }
                        .sealed(),
                        crate::arena::state::CustodyUnit {
                            controller: Controller::Mind,
                            ..tank(UNARMED, PartyId::BLUE, pose(6_500, 6_500))
                        }
                        .sealed(),
                    ],
                    Vec::new(),
                ),
                &cfg,
                &crate::config::MatchConfig::default(),
                &crate::config::SensorsConfig::default(),
            );
            assert!(opened.is_ok(), "the match opens: {opened:?}");
        }
        let mirror = tw
            .world()
            .module_state::<ArenaRegistry>()
            .and_then(|registry| registry.mirror(MATCH))
            .cloned()
            .expect("the mirror row");
        assert_eq!(
            mirror.armed.iter().copied().collect::<Vec<ObjId>>(),
            vec![ARMED]
        );
        let mut said = |line: &str| -> Vec<String> {
            tw.run(GOD, line)
                .into_iter()
                .filter_map(|(audience, event)| match (audience, event) {
                    (mush_core::Audience::Actor(GOD), mush_proto::OutputEvent::Sys(text)) => {
                        Some(text.to_plain_text())
                    }
                    _ => None,
                })
                .collect()
        };
        let armed = said(&format!("@mind/live #{}", ARMED.0));
        assert_eq!(armed.len(), 6, "{armed:?}");
        assert!(
            armed
                .first()
                .is_some_and(|head| head.ends_with(": Line (as checked out)")),
            "{armed:?}"
        );
        assert!(
            armed
                .get(2)
                .is_some_and(|line| line.starts_with("  fire discipline  min_odds 10%")),
            "{armed:?}"
        );
        let unarmed = said(&format!("@mind/live #{}", UNARMED.0));
        assert_eq!(unarmed.len(), 6, "{unarmed:?}");
        assert_eq!(
            unarmed.get(2).map(String::as_str),
            Some("  fire discipline  no weapons station on this hull"),
            "{unarmed:?}"
        );
        let help = tw
            .engine_mut()
            .registry
            .spec_of("@mind")
            .map(|spec| spec.help.clone())
            .expect("@mind is registered");
        assert!(
            help.lines()
                .any(|line| line.starts_with("@mind/live <unit>") && line.contains("(staff)")),
            "{help}"
        );
        assert!(help.contains("@mind/set <name>/<profile>"), "{help}");
        assert!(help.contains("@mind/check <name>"), "{help}");
        assert!(help.contains("as checked out"), "{help}");
        // 17g C2a: the author's commerce commands, directly after /localcopy.
        let localcopy = help
            .lines()
            .position(|line| line.starts_with("@mind/localcopy "))
            .expect("the /localcopy line");
        assert_eq!(
            help.lines().skip(localcopy + 1).take(3).collect::<Vec<_>>(),
            [
                "@mind/rate <name>@<rev>; @mind/file <name>@<rev>[=<charter name>] (Registry rating, then filing)",
                "@mind/update <filed> <name>@<rev>; @mind/withdraw <filed>; @mind/attest <filed> [org=<org>]",
                "@mind/fork <licence>=<name> (from an Open licence; @mind/localcopy copies your own drafts)",
            ],
            "{help}"
        );
        // 17g C2c: `@mind/publish`, directly after the C2a lines.
        assert_eq!(
            help.lines().nth(localcopy + 4),
            Some(
                "@mind/publish <filed> form=sealed|open|open-resale [royalty=<per mille>] [transfer=no] (at a counter)"
            ),
            "{help}"
        );
        let tasks = tw
            .world()
            .module_state::<ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .expect("the registry is composed");
        if let Some((handle, join)) = tasks.remove(MATCH) {
            handle.signal_stop();
            join.abort();
        }
    }

    /// **16b cut O.2** (the cut O plan's item 38): the pod-switch hint's stamp,
    /// read and expiry. It answers the switch last forwarded while it stands —
    /// through the stamped second itself — a later stamp replaces an earlier
    /// one, it answers `None` past the second and for a pilot never stamped,
    /// and an unbind drops it with the seat.
    #[test]
    fn o2_the_emcon_hint_is_stamped_read_and_expires() {
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::default())]);
        let pilot = tw.spawn_player("Ada");
        let other = tw.spawn_player("Bo");
        let registry = tw
            .world_mut()
            .module_state_mut::<ArenaRegistry>()
            .expect("the registry is composed");
        assert_eq!(registry.emcon_hint(pilot, 100), None, "no stamp, no hint");
        registry.hint_emcon(pilot, false, 106);
        assert_eq!(registry.emcon_hint(pilot, 100), Some(false));
        assert_eq!(
            registry.emcon_hint(pilot, 106),
            Some(false),
            "the stamped second stands"
        );
        assert_eq!(
            registry.emcon_hint(pilot, 107),
            None,
            "and the next does not"
        );
        assert_eq!(registry.emcon_hint(other, 100), None, "per pilot");
        // Off then on inside one pulse: the later stamp is the one read.
        registry.hint_emcon(pilot, true, 108);
        assert_eq!(registry.emcon_hint(pilot, 107), Some(true));
        registry.clear_emcon_hint(pilot);
        assert_eq!(registry.emcon_hint(pilot, 107), None, "cleared");
        registry.hint_emcon(pilot, true, 108);
        registry.unbind_pilot(pilot);
        assert_eq!(
            registry.emcon_hint(pilot, 107),
            None,
            "unbound with the seat"
        );
    }

    /// **16b cut AP.3's two rows, at the registry** (16c N19): the switch is
    /// per pilot; `note_watch_block` records a new block and refuses the same
    /// text; `WATCH OFF` forgets the last block, so the next is new again; and
    /// the switch with its last block goes with the seat (`unbind_pilot`) and
    /// with the match (`untrack`) — the second for a pilot whose seat never
    /// reached `Released`, so a cleared cockpit never leaks a digest into the
    /// next match.
    #[test]
    fn ap3_the_line_watch_is_per_pilot_compares_its_block_and_goes_with_the_seat_and_the_match() {
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(crate::TacsimRuleset::default())]);
        let pilot = tw.spawn_player("Ada");
        let other = tw.spawn_player("Bo");
        let unit = tw.spawn_player("Tank");
        let registry = tw
            .world_mut()
            .module_state_mut::<ArenaRegistry>()
            .expect("the registry is composed");
        assert!(!registry.line_watching(pilot), "every seat starts off");
        registry.set_line_watch(pilot, true);
        assert!(registry.line_watching(pilot));
        assert!(!registry.line_watching(other), "per pilot");

        // The comparison: new, the same text, then different text.
        assert!(registry.note_watch_block(pilot, "Watch RA: online, 0 kph, heading 090°"));
        assert!(
            !registry.note_watch_block(pilot, "Watch RA: online, 0 kph, heading 090°"),
            "the same block was taken as new"
        );
        assert!(registry.note_watch_block(pilot, "Watch RA: online, 4 kph, heading 090°"));
        assert!(
            registry.note_watch_block(other, "Watch RA: online, 4 kph, heading 090°"),
            "one pilot's last block is not another's"
        );

        // Off forgets the last block: on again, the same text is new.
        registry.set_line_watch(pilot, false);
        assert!(!registry.line_watching(pilot));
        registry.set_line_watch(pilot, true);
        assert!(
            registry.note_watch_block(pilot, "Watch RA: online, 4 kph, heading 090°"),
            "WATCH OFF kept the last block"
        );

        // The seat's end: `unbind_pilot` drops the switch and the block.
        registry.unbind_pilot(pilot);
        assert!(!registry.line_watching(pilot), "unbound with the seat");
        registry.set_line_watch(pilot, true);
        assert!(
            registry.note_watch_block(pilot, "Watch RA: online, 4 kph, heading 090°"),
            "the unbound seat kept its last block"
        );
        registry.set_line_watch(pilot, false);

        // The match's end, for a pilot still bound to it: `untrack` drops both.
        const MATCH: MatchId = MatchId(78);
        registry.track(MATCH);
        registry.bind_pilot(pilot, MATCH, unit);
        registry.set_line_watch(pilot, true);
        assert!(registry.note_watch_block(pilot, "Watch RA: online, 4 kph, heading 090°"));
        // A pilot of another match keeps theirs.
        registry.track(MatchId(79));
        registry.bind_pilot(other, MatchId(79), unit);
        registry.set_line_watch(other, true);
        registry.untrack(MATCH);
        assert!(
            !registry.line_watching(pilot),
            "the cleared match left the switch on"
        );
        assert!(
            registry.note_watch_block(pilot, "Watch RA: online, 4 kph, heading 090°"),
            "the cleared match kept the last block"
        );
        assert!(
            registry.line_watching(other),
            "another match's pilot was cleared"
        );
        assert!(
            !registry.note_watch_block(other, "Watch RA: online, 4 kph, heading 090°"),
            "another match's pilot lost their last block"
        );
    }

    /// **The sources are keyed by the SEAL's digest** (17b cut M3.5; doc 17
    /// §13): a scripted seal's text, looked up by its `source_sha256`, is
    /// handed on under `MindSeal::digest` — the digest the arena's
    /// `UnitState::seal` carries — so the installer finds it for the hull
    /// sealed with it; a native seal names nothing, and a scripted seal whose
    /// source the store lacks is left out (the installer refuses it by name).
    #[test]
    fn m3_the_sources_are_keyed_by_the_seal_s_digest() {
        use crate::arena::MindSeal;
        use crate::arena::mind::SealEngine;
        use crate::minds::{STANDARD_RHAI, install_sealed_runners, standard_digest};
        let stock = standard_digest().0;
        let scripted = MindSeal {
            unit: ObjId(1),
            source_sha256: stock,
            engine: SealEngine::Scripted,
            ..MindSeal::placeholder()
        };
        let native = MindSeal {
            unit: ObjId(2),
            source_sha256: stock,
            engine: SealEngine::Native,
            ..MindSeal::placeholder()
        };
        let missing = MindSeal {
            unit: ObjId(3),
            source_sha256: [5; 32],
            engine: SealEngine::Scripted,
            ..MindSeal::placeholder()
        };
        let sources = sources_by_seal([&scripted, &native, &missing].into_iter(), |sha256| {
            (*sha256 == stock).then_some(STANDARD_RHAI)
        });
        assert_eq!(
            sources.keys().copied().collect::<Vec<[u8; 32]>>(),
            vec![scripted.digest()],
            "one key, the scripted seal's digest"
        );
        assert_ne!(scripted.digest(), stock);
        assert_eq!(
            sources.get(&scripted.digest()).map(String::as_str),
            Some(STANDARD_RHAI)
        );

        let mut arena = crate::arena::test_support::arena_of(vec![
            tank(ObjId(1), PartyId::RED, pose(1_500, 1_500)),
            tank(ObjId(2), PartyId::BLUE, pose(6_500, 6_500)),
        ]);
        if let Some(unit) = arena.unit_mut(ObjId(1)) {
            unit.seal = scripted.seal_ref();
        }
        let cfg = arena.arena_cfg.clone();
        assert_eq!(
            install_sealed_runners(&mut arena, &cfg, &|digest| sources.get(digest).cloned()),
            Ok(1),
            "the installer finds the source by the arena's seal reference"
        );
    }

    /// **The stock source resolves with no store** (17b cut M4.3; plan
    /// (iii)): `sealed_sources` over a world that holds no `TacsimStore` at
    /// all answers Standard Doctrine's text for a scripted seal on the stock
    /// digest, keyed by the seal's digest — so a world whose store is off
    /// ran and resumed every production seal from 17b cut M4.3 to the fill
    /// decision (2026-10-09), and resumed a pre-decision match's
    /// stock-scripted seals since (a production seal is `native` since and
    /// names no source) until 17b cut M4.6's format 9, which voids every
    /// older snapshot — and nothing for another.
    #[test]
    fn m4_3_the_stock_source_resolves_with_no_store() {
        use crate::arena::MindSeal;
        use crate::arena::mind::SealEngine;
        use crate::minds::{STANDARD_RHAI, standard_digest};
        let tw = mush_testkit::TestWorld::with_modules(vec![]);
        assert!(
            tw.world()
                .module_state::<crate::store::TacsimStore>()
                .is_none(),
            "no store at all"
        );
        let stock = MindSeal {
            unit: ObjId(1),
            source_sha256: standard_digest().0,
            engine: SealEngine::Scripted,
            ..MindSeal::placeholder()
        };
        let other = MindSeal {
            unit: ObjId(2),
            source_sha256: [5; 32],
            engine: SealEngine::Scripted,
            ..MindSeal::placeholder()
        };
        let sources = super::sealed_sources(tw.world(), [&stock, &other].into_iter());
        assert_eq!(
            sources.keys().copied().collect::<Vec<[u8; 32]>>(),
            vec![stock.digest()],
            "the stock seal alone"
        );
        assert_eq!(
            sources.get(&stock.digest()).map(String::as_str),
            Some(STANDARD_RHAI)
        );
    }
}

#[cfg(test)]
mod bulk_commit_tests {
    use super::*;
    #[test]
    fn a_prepaid_lease_waits_for_its_exact_durable_batch() {
        let tasks = ArenaTasks::new(2);
        let latch = crate::minds::bulk::commit::CommitLatch::default();
        tasks.await_bulk_write(latch.clone());
        tasks.ack_ledger_upto(100);
        assert!(!latch.committed(), "an unstamped write never runs");
        tasks.stamp_ledger_writes(101);
        tasks.ack_ledger_upto(100);
        assert!(!latch.committed());
        tasks.ack_ledger_upto(101);
        assert!(latch.committed());
    }

    #[test]
    fn wg4_map_query_draft_branch_is_staff_gated() {
        use mush_core::GOD;
        use mush_core::module::QueryHandler;

        let mut tw = mush_testkit::TestWorld::with_modules(vec![]);
        let player = tw.spawn_player("Player0");
        let params = serde_json::json!({ "draft": "proving_ground" });

        let mush_proto::OutputEvent::Panel { data: denied, .. } =
            MapQuery.query(tw.world(), player, &params).expect("query")
        else {
            panic!("expected Panel");
        };
        assert_eq!(denied["error"], "Permission denied.");

        let mush_proto::OutputEvent::Panel { data: allowed, .. } =
            MapQuery.query(tw.world(), GOD, &params).expect("query")
        else {
            panic!("expected Panel");
        };
        assert_eq!(allowed["schema"], "ground.draft.v1");
        assert_eq!(allowed["id"], "proving_ground");
    }
}
