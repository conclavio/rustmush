//! The single-writer world task (design 01 §4.1).
//!
//! One tokio task owns the [`Engine`] (and therefore the `World`) outright —
//! zero locks on game state. Everything else sends [`WorldMsg`] over an mpsc
//! and receives output through the [`SessionManager`].
//!
//! Each loop iteration:
//!
//! 1. `select!` over the inbound channel and a sleep until the scheduler's
//!    next deadline ([`Scheduler::next_due`](mush_core::Scheduler::next_due));
//! 2. feed wall time in ([`Engine::set_now`]), then [`Engine::step`] (or
//!    [`Engine::fire_due`] on a timer wake);
//! 3. **deliver**: resolve each queued [`Audience`] against world state
//!    (room → connected actor ids — treaty ruling 2's world half) and hand
//!    per-character sends to the `SessionManager` (its connection half),
//!    `Arc`-wrapping each event once;
//! 4. **spawn effects**: every [`Effect`] drained from the engine becomes a
//!    detached tokio task whose completion re-enters as
//!    [`Inbound::Apply`]`(`[`WorldCommand`]`)` (treaty ruling 3).
//!
//! Account-service requests ([`WorldMsg::CreatePlayer`]) arrive as typed
//! messages with a oneshot reply, so `accounts.rs` never touches `World`.
//!
//! ## WHO wiring (decision)
//!
//! `WHO` inside `mush-core` reads the engine-maintained per-actor session
//! stats (`World::session_stat`), which this task keeps real by feeding wall
//! time before every step and by the engine stamping `last_input` on each
//! [`Inbound::Cmd`] and `connect_time` on the first `ConnUp`. That keeps
//! `mush-core` completely ignorant of `mush-session` — no snapshot pushing,
//! no trait threading — and WHO's idle/connect columns are exact. (The
//! session manager remains the authority on peer addresses; the M1 WHO
//! format does not show them.)

use std::sync::Arc;
use std::time::{Duration, Instant};

use mush_core::{
    Audience, DisconnectKind, Effect, Effects, Engine, Inbound, ObjId, ObjKind, Outbox, Registry,
    StepOutcome, World, WorldCommand, register_core_commands, register_engine_commands,
};
use mush_proto::OutputEvent;
use mush_session::{CharKey, ConnId as SessConnId, DetachOutcome, DetachReason, SessionManager};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::{unix_now_ms, unix_now_s};

/// Capacity of the world task's inbound channel.
const INBOUND_CAPACITY: usize = 1024;

/// Player creation failure, reported back over the oneshot.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CreatePlayerErr {
    /// An actor with that name (or alias) already exists.
    #[error("that name is already in use")]
    NameTaken,
    /// Any other world error (invariant violation — should not happen).
    #[error("world error: {0}")]
    World(String),
    /// The world task is gone (shutdown).
    #[error("the world is shutting down")]
    WorldGone,
}

/// Everything the world task can receive: engine inbound traffic plus typed
/// service requests from async subsystems that must not touch `World`.
pub enum WorldMsg {
    /// Ordinary engine input (commands, connection lifecycle, applies,
    /// control).
    Inbound(Inbound),
    /// Create a Player object (account creation): the world task creates it
    /// in the start room and replies with the new id.
    CreatePlayer {
        /// The character name.
        name: String,
        /// Where the result goes.
        reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
    },
    /// Ordering barrier: replied to once every message queued before it has
    /// been processed *and its output delivered*. The QUIT flow uses this to
    /// flush a quitting player's in-flight output before detaching them.
    Barrier {
        /// Signalled when the barrier is reached.
        reply: oneshot::Sender<()>,
    },
}

/// Cloneable handle for sending into the world task.
#[derive(Clone)]
pub struct WorldHandle {
    tx: mpsc::Sender<WorldMsg>,
}

impl WorldHandle {
    /// Send one engine inbound message; `false` if the world task is gone.
    pub async fn inbound(&self, msg: Inbound) -> bool {
        self.tx.send(WorldMsg::Inbound(msg)).await.is_ok()
    }

    /// Ask the world task to create a Player named `name`.
    pub async fn create_player(&self, name: &str) -> Result<ObjId, CreatePlayerErr> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::CreatePlayer {
                name: name.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return Err(CreatePlayerErr::WorldGone);
        }
        rx.await.unwrap_or(Err(CreatePlayerErr::WorldGone))
    }

    /// Round-trip barrier: resolves once everything sent through this handle
    /// before the call has been processed and its output delivered. `false`
    /// if the world task is gone.
    pub async fn barrier(&self) -> bool {
        let (reply, rx) = oneshot::channel();
        if self.tx.send(WorldMsg::Barrier { reply }).await.is_err() {
            return false;
        }
        rx.await.is_ok()
    }
}

/// Boot the standard M1 engine: minimal world (Limbo `#0`, God `#1`) with
/// core + engine commands registered and the clock set to now.
///
/// Deliberately *not* `mush_testkit::boot_world` — the testkit is a dev
/// tool; the server owns its own boot wiring (and additionally registers
/// the engine commands `@slowtest` / `@wait`).
///
/// # Panics
///
/// Panics if command registration collides — a programming error.
#[must_use]
pub fn boot_engine() -> Engine {
    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    let mut registry = Registry::new();
    register_core_commands(&mut registry).expect("core command registration must not collide");
    register_engine_commands(&mut registry).expect("engine command registration must not collide");
    Engine::new(world, registry)
}

/// Spawn the world task. Returns the handle everything sends through and the
/// join handle `main` awaits at shutdown.
///
/// The task exits when [`Inbound::Control`]`(Shutdown)` steps to
/// [`StepOutcome::Shutdown`] or when every [`WorldHandle`] (and internal
/// effect sender) is dropped.
pub fn spawn(engine: Engine, sessions: Arc<SessionManager>) -> (WorldHandle, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let handle = WorldHandle { tx: tx.clone() };
    // The loop keeps only a weak sender for effect completions — holding a
    // strong one would keep its own channel open forever, so dropping every
    // WorldHandle could never end the task.
    let join = tokio::spawn(run(engine, sessions, rx, tx.downgrade()));
    (handle, join)
}

/// One `select!` wake: a message, a due timer, or channel closure.
enum Wake {
    Msg(WorldMsg),
    Timer,
    Closed,
}

async fn run(
    mut engine: Engine,
    sessions: Arc<SessionManager>,
    mut rx: mpsc::Receiver<WorldMsg>,
    tx: mpsc::WeakSender<WorldMsg>,
) {
    tracing::info!("world task running");
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    loop {
        let next_due = engine.sched.next_due();
        let wake = tokio::select! {
            m = rx.recv() => m.map_or(Wake::Closed, Wake::Msg),
            () = sleep_until_unix_ms(next_due), if next_due.is_some() => Wake::Timer,
        };
        engine.set_now(unix_now_s());
        let outcome = match wake {
            Wake::Msg(WorldMsg::Inbound(msg)) => engine.step(msg, &mut out, &mut fx),
            Wake::Msg(WorldMsg::CreatePlayer { name, reply }) => {
                let result = create_player(&mut engine, &name);
                // A dropped receiver just means the requester gave up.
                let _ = reply.send(result);
                StepOutcome::Continue
            }
            Wake::Msg(WorldMsg::Barrier { reply }) => {
                // Every earlier message was processed (and delivered) in its
                // own iteration; reaching here *is* the barrier.
                let _ = reply.send(());
                StepOutcome::Continue
            }
            Wake::Timer => {
                engine.fire_due(unix_now_ms(), &mut out, &mut fx);
                StepOutcome::Continue
            }
            Wake::Closed => {
                tracing::info!("world inbound channel closed; world task exiting");
                break;
            }
        };
        deliver_and_police(&mut engine, &sessions, &mut out, &mut fx, &tx);
        spawn_effects(&mut fx, &tx);
        if outcome == StepOutcome::Shutdown {
            tracing::info!("world task shut down cleanly");
            break;
        }
    }
}

/// Sleep until `due` (unix milliseconds). Only ever polled when `due` is
/// `Some` (the select guard); an already-past deadline yields immediately.
///
/// Deadlines are wall-clock unix ms armed against a monotonic sleep: a
/// backward NTP step delays timers by the step size (the loop re-arms
/// harmlessly each iteration), a forward step fires them early. Accepted
/// for M1; revisit if `@wait` precision ever matters.
async fn sleep_until_unix_ms(due: Option<i64>) {
    let Some(due) = due else { return };
    let delta = due.saturating_sub(unix_now_ms()).max(0);
    tokio::time::sleep(Duration::from_millis(u64::try_from(delta).unwrap_or(0))).await;
}

/// Deliver the outbox, then apply the slow-client policy (design 02 §8.4):
/// every connection whose buffer overflowed is detached as
/// [`DetachReason::SlowClient`] and its `ConnDown` is fed straight back into
/// the engine (we *are* the world task, so no channel round-trip — the
/// resulting link-dead announcement is delivered in the same pass). Repeats
/// until a delivery pass drops nothing.
fn deliver_and_police(
    engine: &mut Engine,
    sessions: &Arc<SessionManager>,
    out: &mut Outbox,
    fx: &mut Effects,
    tx: &mpsc::WeakSender<WorldMsg>,
) {
    let mut slow = deliver(&engine.world, sessions, out);
    while !slow.is_empty() {
        for conn in std::mem::take(&mut slow) {
            disconnect_slow_client(engine, sessions, conn, out, fx, tx);
        }
        slow = deliver(&engine.world, sessions, out);
    }
}

/// Detach one overflowed connection and tell the engine. Dropping the
/// session's sender also makes the transport's output pump close the socket
/// once it drains (the pump additionally bounds each write with a timeout,
/// so a wedged transport cannot hold the queue open forever).
fn disconnect_slow_client(
    engine: &mut Engine,
    sessions: &Arc<SessionManager>,
    conn: SessConnId,
    out: &mut Outbox,
    fx: &mut Effects,
    tx: &mpsc::WeakSender<WorldMsg>,
) {
    // Resolve the actor before the detach forgets the mapping.
    let actor = sessions
        .session_of_conn(conn)
        .and_then(|s| sessions.character_of(s))
        .map(char_key_obj);
    let Some(outcome) = sessions.detach(conn, DetachReason::SlowClient) else {
        return; // already detached (duplicate report or concurrent close)
    };
    tracing::warn!(%conn, "slow client disconnected (output buffer overflow)");
    let Some(actor) = actor else { return };
    let reason = match outcome {
        // Session already gone underneath us: clean full disconnect.
        DetachOutcome::Closed => DisconnectKind::Boot,
        DetachOutcome::OthersRemain | DetachOutcome::Linkdead { .. } => DisconnectKind::Lost,
    };
    engine.step(
        Inbound::ConnDown {
            conn: conn.0,
            actor,
            reason,
        },
        out,
        fx,
    );
    if let DetachOutcome::Linkdead { grace } = outcome {
        arm_linkdead_reaper(sessions, tx, grace);
    }
}

/// After `grace`, reap every expired link-dead session and send its
/// `SessionGone` (the engine re-validates against live connections, so a
/// reconnect that raced the reaper is never clobbered).
fn arm_linkdead_reaper(
    sessions: &Arc<SessionManager>,
    tx: &mpsc::WeakSender<WorldMsg>,
    grace: Duration,
) {
    let sessions = Arc::clone(sessions);
    let tx = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        for expired in sessions.take_expired(Instant::now()) {
            let actor = char_key_obj(expired.character);
            tracing::info!(%actor, account = %expired.account, "linkdead grace expired");
            let Some(tx) = tx.upgrade() else { return };
            if tx
                .send(WorldMsg::Inbound(Inbound::SessionGone { actor }))
                .await
                .is_err()
            {
                return;
            }
        }
    });
}

/// Create a Player in the start room (Limbo in M1). Players own themselves.
fn create_player(engine: &mut Engine, name: &str) -> Result<ObjId, CreatePlayerErr> {
    match engine.world.create(
        ObjKind::Player,
        name,
        mush_core::GOD,
        Some(mush_core::LIMBO),
    ) {
        Ok(id) => {
            tracing::info!(%id, name, "created player");
            Ok(id)
        }
        Err(mush_core::WorldError::NameInUse(_)) => Err(CreatePlayerErr::NameTaken),
        Err(e) => Err(CreatePlayerErr::World(e.to_string())),
    }
}

/// Resolve every queued audience to per-character (or per-connection) sends
/// through the session manager. The world half of treaty ruling 2 lives
/// here: `Room` audiences become the room's *connected actors*; the session
/// manager takes it from character to connections.
///
/// Returns the connections whose buffer overflowed (the event was dropped
/// for them); the caller disconnects them as slow clients.
fn deliver(world: &World, sessions: &SessionManager, out: &mut Outbox) -> Vec<SessConnId> {
    let mut slow = Vec::new();
    for (audience, event) in out.take() {
        let ev = Arc::new(event);
        match audience {
            Audience::Actor(actor) => {
                let report = sessions.send_to_char(char_key(actor), ev);
                if report.delivered == 0 && report.dropped.is_empty() {
                    // Link-dead actor (or a race with a disconnect): the
                    // event is lost. Resume buffers are an M2+ concern
                    // (PLAN: mush-session "resume buffers"); make the loss
                    // observable at least.
                    tracing::debug!(%actor, "output to actor with no live connections dropped");
                }
                slow.extend(report.dropped);
            }
            Audience::Conn(conn) => {
                // Pre-auth / connection-addressed events. An unknown conn is
                // fine (it may have raced a disconnect).
                let _ = sessions.send_to_conn(SessConnId(conn), ev);
            }
            Audience::Room { room, exclude } => {
                for &occupant in world.contents_of(room) {
                    if exclude.contains(&occupant) || !world.is_connected(occupant) {
                        continue;
                    }
                    let report = sessions.send_to_char(char_key(occupant), Arc::clone(&ev));
                    slow.extend(report.dropped);
                }
            }
            Audience::All => {
                let report = sessions.broadcast_all(ev);
                slow.extend(report.dropped);
            }
        }
    }
    slow.sort_unstable();
    slow.dedup();
    slow
}

/// Spawn one detached task per drained [`Effect`]; each completion re-enters
/// the world as a typed [`WorldCommand`] via [`Inbound::Apply`].
///
/// Each in-flight effect holds a strong sender (upgraded from the loop's
/// weak one), so the world task outlives every pending completion; once the
/// last handle *and* the last effect are gone, the channel closes and the
/// task ends.
fn spawn_effects(fx: &mut Effects, tx: &mpsc::WeakSender<WorldMsg>) {
    for effect in fx.drain() {
        match effect {
            Effect::SlowTest {
                actor,
                correlation,
                seconds,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(correlation, "world channel closing; slowtest dropped");
                    continue;
                };
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                    let done = WorldCommand::SlowTestDone {
                        actor,
                        correlation,
                        text: format!(
                            "SLOWTEST: fake async call #{correlation} finished after \
                             {seconds}s -- the world loop never blocked."
                        ),
                    };
                    if tx
                        .send(WorldMsg::Inbound(Inbound::Apply(done)))
                        .await
                        .is_err()
                    {
                        tracing::debug!(correlation, "slowtest completion after shutdown dropped");
                    }
                });
            }
        }
    }
}

/// Widen a core [`ObjId`] to the session layer's opaque [`CharKey`].
#[must_use]
pub fn char_key(id: ObjId) -> CharKey {
    CharKey(u64::from(id.0))
}

/// Narrow a [`CharKey`] back to the core [`ObjId`] it was widened from.
#[must_use]
pub fn char_key_obj(key: CharKey) -> ObjId {
    ObjId(u32::try_from(key.0).unwrap_or(u32::MAX))
}

/// Deliver one event straight to a connection, bypassing the world task
/// (login screens, error notices). Used by the SSH bridge.
pub fn send_event_to_conn(sessions: &SessionManager, conn: SessConnId, event: OutputEvent) {
    let _ = sessions.send_to_conn(conn, Arc::new(event));
}
