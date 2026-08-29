//! The single-writer world task (design 01 §4.1) plus the M2 write-through
//! persistence seam.
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
//!    [`Inbound::Apply`]`(`[`WorldCommand`]`)` (treaty ruling 3);
//! 5. **write through** (M2): whatever the step dirtied is snapshotted via
//!    `World::take_dirty()` into ONE [`WriteBatch`](mush_store::WriteBatch)
//!    and sent to the store-writer task immediately — no timer batching
//!    (`@destroy`ed husks are swept to tombstones first, so the same batch
//!    carries their store deletes). Ordinary command batches are
//!    fire-and-forget (the writer retries transient failures forever, in
//!    order); durability-critical flows carry a receipt oneshot and their
//!    confirmations wait for the ack — Player creation via its typed reply,
//!    and any step whose handler called `Ctx::gate_on_durable_ack` (mail
//!    send, job approval) via a held-output gate: the step's resolved
//!    output is transmitted only once the store acks its durable batch;
//! 6. **staff ops** (M2): `@siteban`/`@dump`/`@shutdown` handlers
//!    (registered by [`crate::staff`]) queued typed ops during the step;
//!    ban ops are executed by ONE sequential executor task (staff commands
//!    issued in order can never race in the store), `@dump` rides the
//!    store-writer queue, and all are ack-gated and audit-logged
//!    (`@shutdown` instead makes the loop step `ControlMsg::Shutdown` and
//!    exit, same as Ctrl-C/SIGTERM).
//!
//! ## Pause-mutations policy (PLAN risk 3)
//!
//! When the store-writer reports persistent write failure
//! ([`WorldMsg::StoreStatus`]), the world enters **read-only mode**: input
//! lines are refused unless they are on a conservative non-mutating
//! allowlist (look/WHO/speech/...) — a line matching an **exit** is refused
//! even when it spells an allowlisted verb, since dispatch matches exits
//! first and traversal mutates — and connected Wizards get a Sys alert.
//! [`WorldMsg::CreatePlayer`] is refused outright while read-only. The
//! writer keeps retrying the failed batch with backoff; on recovery the
//! mode lifts and Wizards are told. RAM never diverges beyond what is
//! already queued (in order) for the writer.
//!
//! Account-service requests ([`WorldMsg::CreatePlayer`]) arrive as typed
//! messages with a oneshot reply, so `accounts.rs` never touches `World`.
//! The reply is **deferred until the creation batch is acknowledged by the
//! store** — that is the ack gate account/character creation confirmation
//! output rides on.
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

use mush_core::commands::movement::find_exit;
use mush_core::{
    Audience, ControlMsg, DisconnectKind, Effect, Effects, Engine, Flags, Inbound, ObjId, ObjKind,
    Outbox, Registry, Role, ScheduledKind, StepOutcome, TickCadence, World, WorldCommand,
    WorldEvent, is_ignoring, narrative_actor, register_core_commands, register_engine_commands,
};
use mush_llm::LlmService;
use mush_proto::{OutputEvent, RichText};
use mush_session::{CharKey, ConnId as SessConnId, DetachOutcome, DetachReason, SessionManager};
use mush_store::{InMemoryStore, ObjectStore};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::ipguard::IpGuard;
use crate::persist::{self, StoreJob};
use crate::staff::{StaffOp, StaffOpKind, StaffQueue};
use crate::{unix_now_ms, unix_now_s};

/// Capacity of the world task's inbound channel.
const INBOUND_CAPACITY: usize = 1024;

/// Capacity of the sequential staff-op executor's queue (rare wizard
/// commands; the bound only matters as backpressure).
const STAFF_OPS_CAPACITY: usize = 64;

/// One loop iteration (step + delivery + flush) taking longer than this is
/// logged as loop lag — the observability hook PLAN's risk 2 asks for. The
/// world loop must never block on slow work; lag here means something
/// synchronous is too slow or the writer queue is back-pressuring.
const LOOP_LAG_WARN: Duration = Duration::from_millis(500);

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
    /// The store is unavailable (read-only mode): creating a Player would
    /// mutate RAM with no prospect of a timely durable ack, so creation is
    /// refused up front instead of leaving the requester hanging.
    #[error("the database is temporarily unavailable; please try again shortly")]
    StoreUnavailable,
}

/// Everything the world task can receive: engine inbound traffic plus typed
/// service requests from async subsystems that must not touch `World`.
pub enum WorldMsg {
    /// Ordinary engine input (commands, connection lifecycle, applies,
    /// control).
    Inbound(Inbound),
    /// Create a Player object (account/character creation): the world task
    /// creates it in the start room, writes the creation batch through the
    /// store (`durable: true`), and replies with the new id **once the
    /// store has acknowledged the batch** — the ack gate for creation
    /// confirmations.
    CreatePlayer {
        /// The character name.
        name: String,
        /// Where the result goes.
        reply: oneshot::Sender<Result<ObjId, CreatePlayerErr>>,
    },
    /// Is this actor name (player index, case-insensitive) unclaimed?
    /// Used by account creation to pre-check the character name before the
    /// account record is reserved (the create itself still re-validates).
    NameFree {
        /// The name to probe.
        name: String,
        /// `true` = free.
        reply: oneshot::Sender<bool>,
    },
    /// Ordering barrier: replied to once every message queued before it has
    /// been processed *and its output delivered*. The QUIT flow uses this to
    /// flush a quitting player's in-flight output before detaching them.
    Barrier {
        /// Signalled when the barrier is reached.
        reply: oneshot::Sender<()>,
    },
    /// Store-writer health transition (sent by the writer task): `false`
    /// enters read-only mode (mutations paused, Wizards alerted), `true`
    /// lifts it.
    StoreStatus {
        /// Whether writes are succeeding again.
        healthy: bool,
        /// The triggering error, for the staff alert (empty on recovery).
        detail: String,
    },
}

/// The world loop's in-task mirror of store-writer health — the small
/// `StoreStatus` mirror `@stats` reads (the writer task itself only sends
/// health *transitions*; the loop keeps the current picture).
#[derive(Clone, Copy, Debug)]
pub struct StoreHealth {
    /// Whether the world is in read-only mode (writes failing).
    pub read_only: bool,
    /// Unix seconds when the most recent batch was handed to the writer
    /// (`None` before the first write of this boot).
    pub last_write_unix: Option<i64>,
}

impl StoreHealth {
    /// Healthy, nothing written yet.
    #[must_use]
    pub fn new() -> StoreHealth {
        StoreHealth {
            read_only: false,
            last_write_unix: None,
        }
    }
}

impl Default for StoreHealth {
    fn default() -> Self {
        StoreHealth::new()
    }
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

    /// Ask the world task to create a Player named `name`. Resolves only
    /// after the creation batch is acknowledged by the store.
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

    /// Whether an actor name is currently unclaimed. `false` when the world
    /// task is gone (conservative).
    pub async fn name_free(&self, name: &str) -> bool {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(WorldMsg::NameFree {
                name: name.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
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

/// The server-side dependencies the world task drives besides the engine:
/// the store (staff ops, bgsave), the IP guard (siteban mirror), and the
/// staff-command queue shared with the registry handlers.
pub struct ServerDeps {
    /// The persistence backend.
    pub store: Arc<dyn ObjectStore>,
    /// The shared per-IP guard (its siteban mirror is updated here).
    pub guard: Arc<IpGuard>,
    /// The queue `register_server_commands` handlers push into.
    pub staff: Arc<StaffQueue>,
    /// The M7 LLM service handle. Present in the real server (built from
    /// `[llm]` config); [`spawn_effects`] uses it to run a module's deferred
    /// [`Effect::ModuleAsync`] (worldgen `@gen`) off the world loop and
    /// re-enter as [`WorldCommand::ModuleAsyncDone`]. `None` for the compat
    /// [`spawn`] and lightweight tests that boot no LLM-driven module — such
    /// an effect is then logged and dropped, never a panic.
    pub llm: Option<Arc<LlmService>>,
}

impl ServerDeps {
    /// Throwaway deps for tests and the compat [`spawn`]: fresh in-memory
    /// store, fresh guard, fresh (never-populated) staff queue, no LLM.
    #[must_use]
    pub fn in_memory() -> ServerDeps {
        ServerDeps {
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
            llm: None,
        }
    }
}

/// Handles from [`spawn_persist`]: the sender everything talks through and
/// the join handles the shutdown sequence awaits **in order** (world first,
/// then the writer — dropping the world task's job sender is what lets the
/// writer drain and exit).
pub struct PersistHandles {
    /// Sender handle for the world task.
    pub handle: WorldHandle,
    /// The world task itself.
    pub world_join: JoinHandle<()>,
    /// The store-writer task (drains after the world task ends).
    pub writer_join: JoinHandle<()>,
}

/// Boot the standard engine: minimal world (Limbo `#0`, God `#1`) with
/// core + engine commands registered and the clock set to now.
///
/// Deliberately *not* `mush_testkit::boot_world` — the testkit is a dev
/// tool; the server owns its own boot wiring (and additionally registers
/// the engine commands `@slowtest` / `@wait`). The real server boot loads
/// the world from the store instead ([`persist::load_or_seed_world`]) and
/// also registers the staff commands
/// ([`crate::staff::register_server_commands`]).
///
/// # Panics
///
/// Panics if command registration collides — a programming error.
#[must_use]
pub fn boot_engine() -> Engine {
    let mut world = World::new_minimal();
    world.set_now(unix_now_s());
    Engine::new(world, base_registry())
}

/// The always-registered command table: core + engine commands.
///
/// # Panics
///
/// Panics if command registration collides — a programming error.
#[must_use]
pub fn base_registry() -> Registry {
    let mut registry = Registry::new();
    register_core_commands(&mut registry).expect("core command registration must not collide");
    register_engine_commands(&mut registry).expect("engine command registration must not collide");
    registry
}

/// Compatibility spawn (tests, tools): the full world task over throwaway
/// in-memory persistence. Returns the handle everything sends through and
/// the world join handle; the writer task is detached (its writes go to a
/// store nobody reads).
pub fn spawn(engine: Engine, sessions: Arc<SessionManager>) -> (WorldHandle, JoinHandle<()>) {
    let handles = spawn_persist(engine, sessions, ServerDeps::in_memory());
    (handles.handle, handles.world_join)
}

/// Spawn the world task and its store-writer. The world task exits when
/// [`Inbound::Control`]`(Shutdown)` steps to [`StepOutcome::Shutdown`] or
/// when every [`WorldHandle`] (and internal effect sender) is dropped; the
/// writer drains its queue and exits after that.
pub fn spawn_persist(
    engine: Engine,
    sessions: Arc<SessionManager>,
    deps: ServerDeps,
) -> PersistHandles {
    let (tx, rx) = mpsc::channel(INBOUND_CAPACITY);
    let handle = WorldHandle { tx: tx.clone() };
    // The writer notifies health transitions through a weak sender so it
    // can never keep the world channel alive on its own.
    let (writer_tx, writer_join) =
        persist::spawn_store_writer(Arc::clone(&deps.store), tx.downgrade());
    // Sequential executor for staff store ops (@siteban set/clear/list):
    // ONE task draining ONE queue, so two staff commands issued in order
    // can never race each other in the store or the IpGuard mirror. It
    // drains and exits when the world loop (the only sender) ends.
    let (staff_tx, staff_rx) = mpsc::channel(STAFF_OPS_CAPACITY);
    // The staff executor gets only a WEAK world sender (like the writer):
    // it needs the world for `@newpassword`'s web-connection boots, but a
    // strong sender would keep the world loop's channel alive while the
    // loop is what closes the staff queue — a shutdown deadlock.
    tokio::spawn(staff_ops_loop(
        Arc::clone(&deps.store),
        Arc::clone(&deps.guard),
        Arc::clone(&sessions),
        tx.downgrade(),
        staff_rx,
    ));
    // The loop keeps only a weak sender for effect completions — holding a
    // strong one would keep its own channel open forever, so dropping every
    // WorldHandle could never end the task.
    let world_join = tokio::spawn(run(
        engine,
        sessions,
        rx,
        tx.downgrade(),
        writer_tx,
        deps,
        staff_tx,
    ));
    PersistHandles {
        handle,
        world_join,
        writer_join,
    }
}

/// One `select!` wake: a message, a due timer, or channel closure.
enum Wake {
    Msg(WorldMsg),
    Timer,
    Closed,
}

#[allow(clippy::too_many_lines)] // the world loop is one cohesive state machine
async fn run(
    mut engine: Engine,
    sessions: Arc<SessionManager>,
    mut rx: mpsc::Receiver<WorldMsg>,
    tx: mpsc::WeakSender<WorldMsg>,
    writer: mpsc::Sender<StoreJob>,
    deps: ServerDeps,
    staff_ops: mpsc::Sender<StaffOp>,
) {
    tracing::info!("world task running");
    let boot_unix = unix_now_s();
    // The LLM service handle (built from [llm] config) that drives a module's
    // deferred async work off-loop; cloned out of `deps` so the loop keeps
    // borrowing `deps` (staff queue) freely each iteration.
    let llm = deps.llm.clone();
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    let mut serial: u64 = 0;
    let mut health = StoreHealth::new();
    loop {
        let next_due = engine.sched.next_due();
        let wake = tokio::select! {
            m = rx.recv() => m.map_or(Wake::Closed, Wake::Msg),
            () = sleep_until_unix_ms(next_due), if next_due.is_some() => Wake::Timer,
        };
        let iteration_start = Instant::now();
        engine.set_now(unix_now_s());
        // Whose flood-exempt flag to re-sync after the step (the acting
        // player of a Cmd — the only path that pushes/pops input contexts).
        let mut flood_sync: Option<ObjId> = None;
        let outcome = match wake {
            Wake::Msg(WorldMsg::Inbound(msg)) => {
                if let Inbound::Cmd { actor, .. } = &msg {
                    flood_sync = Some(*actor);
                }
                if health.read_only
                    && let Some(refused) = refuse_read_only(&engine.world, &msg)
                {
                    let (actor, text) = refused;
                    sessions.send_to_char(
                        char_key(actor),
                        Arc::new(OutputEvent::Sys(RichText::tag("error", text))),
                    );
                    StepOutcome::Continue
                } else {
                    engine.step(msg, &mut out, &mut fx)
                }
            }
            Wake::Msg(WorldMsg::CreatePlayer { name, reply }) => {
                if health.read_only {
                    // A creation now would mutate RAM behind a stuck writer
                    // queue and hang the requester on an ack that cannot
                    // come until the store recovers. Fail closed instead.
                    let _ = reply.send(Err(CreatePlayerErr::StoreUnavailable));
                } else {
                    match create_player(&mut engine, &name) {
                        Ok(id) => {
                            // Tell module observers a Player now exists
                            // (comsys auto-joins the default channel here).
                            // Emitted BEFORE the flush so whatever the
                            // observers persist rides the same durable
                            // creation batch.
                            engine.emit_world_event(
                                &WorldEvent::ObjCreated {
                                    obj: id,
                                    kind: ObjKind::Player,
                                    creator: mush_core::GOD,
                                },
                                &mut out,
                                &mut fx,
                            );
                            // Flush the creation batch (durable) and defer
                            // the reply until the store acks — the whole
                            // point of the gate.
                            match flush_dirty(&mut engine, &writer, &mut serial, true, &mut health)
                                .await
                            {
                                FlushOutcome::Queued(Some(ack_rx)) => {
                                    tokio::spawn(async move {
                                        let result = match ack_rx.await {
                                            Ok(Ok(_)) => Ok(id),
                                            Ok(Err(e)) => Err(CreatePlayerErr::World(format!(
                                                "store write failed: {e}"
                                            ))),
                                            Err(_) => Err(CreatePlayerErr::WorldGone),
                                        };
                                        let _ = reply.send(result);
                                    });
                                }
                                // Nothing dirtied — cannot actually happen
                                // for a create; reply rather than hang.
                                FlushOutcome::Clean | FlushOutcome::Queued(None) => {
                                    let _ = reply.send(Ok(id));
                                }
                                // The writer is gone (teardown): the batch
                                // was DROPPED, so this creation is not
                                // durable — never confirm it.
                                FlushOutcome::WriterGone => {
                                    let _ = reply.send(Err(CreatePlayerErr::WorldGone));
                                }
                            }
                        }
                        Err(e) => {
                            // A dropped receiver: the requester gave up.
                            let _ = reply.send(Err(e));
                        }
                    }
                }
                StepOutcome::Continue
            }
            Wake::Msg(WorldMsg::NameFree { name, reply }) => {
                let _ = reply.send(engine.world.resolve_player(&name).is_none());
                StepOutcome::Continue
            }
            Wake::Msg(WorldMsg::Barrier { reply }) => {
                // Every earlier message was processed (and delivered) in its
                // own iteration; reaching here *is* the barrier.
                let _ = reply.send(());
                StepOutcome::Continue
            }
            Wake::Msg(WorldMsg::StoreStatus { healthy, detail }) => {
                apply_store_status(&engine.world, &sessions, &mut health, healthy, &detail);
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
        // Flood-exempt sync: while the acting player sits in an input
        // context their lines bypass the transport throttle (a pasted mail
        // body must not lose lines 31+ to the burst cap).
        if let Some(actor) = flood_sync {
            sessions.set_flood_exempt(char_key(actor), engine.world.input_depth(actor) > 0);
        }
        if let Some(gate_actor) = engine.take_durable_gate() {
            // The step asked for the durable-ack output gate (mail send,
            // XP approval — PLAN's ack-gated confirmations): resolve the
            // step's audiences against CURRENT world state, flush its
            // batch durable, and transmit the held output only once the
            // store acks. Slow-client policing is skipped for held sends
            // (a handful of lines; drops are still counted by the session
            // layer). Later steps' output may overtake the held lines by
            // up to one ack round-trip — accepted.
            let resolved = resolve_sends(&engine.world, &mut out);
            spawn_effects(&mut fx, &tx, llm.as_ref());
            match flush_dirty(&mut engine, &writer, &mut serial, true, &mut health).await {
                FlushOutcome::Queued(Some(ack_rx)) => {
                    let sessions = Arc::clone(&sessions);
                    tokio::spawn(async move {
                        match ack_rx.await {
                            Ok(Ok(_)) => {
                                let _ = transmit(&sessions, resolved);
                            }
                            Ok(Err(e)) => {
                                // The write was dropped (non-retryable):
                                // deliver what happened in RAM, but never
                                // pretend it is durable.
                                let _ = transmit(&sessions, resolved);
                                send_sys(
                                    &sessions,
                                    gate_actor,
                                    &format!(
                                        "WARNING: the database write for your last \
                                         command failed ({e}); its effects may not \
                                         survive a restart."
                                    ),
                                );
                            }
                            // Teardown: the writer vanished before acking —
                            // the batch may be lost; never confirm.
                            Err(_) => {}
                        }
                    });
                }
                // Nothing was actually dirty: nothing to gate on.
                FlushOutcome::Clean | FlushOutcome::Queued(None) => {
                    let _ = transmit(&sessions, resolved);
                }
                // The batch was DROPPED (writer gone, teardown): the write
                // is not durable — never deliver the confirmation.
                FlushOutcome::WriterGone => {}
            }
        } else {
            deliver_and_police(&mut engine, &sessions, &mut out, &mut fx, &tx);
            spawn_effects(&mut fx, &tx, llm.as_ref());
            // Write-through: one batch per processed message that dirtied
            // the world (fire-and-forget; the writer preserves order and
            // retries).
            let _ = flush_dirty(&mut engine, &writer, &mut serial, false, &mut health).await;
        }
        // Staff ops the step's handlers queued. Local ops (@boot/@ps/@stats)
        // run inline here — the engine, scheduler, and session manager are
        // all at hand — AFTER the delivery above, so a @boot target's
        // Disconnect message reaches their connections before the session
        // is closed. Store ops go to the async executors, as in M2.
        let (local_ops, store_ops): (Vec<StaffOp>, Vec<StaffOp>) = deps
            .staff
            .drain()
            .into_iter()
            .partition(|op| op.kind.is_local());
        if !local_ops.is_empty() {
            run_local_staff_ops(
                &mut engine,
                &sessions,
                &health,
                boot_unix,
                local_ops,
                &mut out,
                &mut fx,
            );
            deliver_and_police(&mut engine, &sessions, &mut out, &mut fx, &tx);
            spawn_effects(&mut fx, &tx, llm.as_ref());
            let _ = flush_dirty(&mut engine, &writer, &mut serial, false, &mut health).await;
        }
        let staff_shutdown = run_staff_ops(&deps, &sessions, &writer, &staff_ops, store_ops).await;
        // Loop-lag watchdog (PLAN risk 2): the single-writer loop must stay
        // fast; a slow iteration means a slow sync handler or writer-queue
        // backpressure. The engine mirrors the numbers for `@stats`.
        let lag = iteration_start.elapsed();
        engine.record_loop_lag(u64::try_from(lag.as_millis()).unwrap_or(u64::MAX));
        if lag > LOOP_LAG_WARN {
            tracing::warn!(
                lag_ms = lag.as_millis() as u64,
                "world loop iteration lagged"
            );
        }
        if outcome == StepOutcome::Shutdown {
            tracing::info!("world task shut down cleanly");
            break;
        }
        if staff_shutdown {
            // In-game `@shutdown`: step the same ControlMsg the console path
            // sends, so the announce-to-all and the StepOutcome are
            // identical; main's shutdown sequence (writer drain → store
            // drain → BGSAVE) runs when this task's join resolves.
            let outcome = engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx);
            deliver_and_police(&mut engine, &sessions, &mut out, &mut fx, &tx);
            spawn_effects(&mut fx, &tx, llm.as_ref());
            let _ = flush_dirty(&mut engine, &writer, &mut serial, false, &mut health).await;
            debug_assert_eq!(outcome, StepOutcome::Shutdown);
            tracing::info!("world task shut down cleanly");
            break;
        }
    }
    // Dropping `writer` here closes the job queue; the writer task drains
    // whatever is still pending and exits (awaited by the shutdown
    // sequence via `PersistHandles::writer_join`).
}

/// In read-only mode, decide whether an inbound message must be refused.
/// Returns `Some((actor, message))` to refuse, `None` to let it through.
///
/// Only `Cmd` lines are ever refused, and only when the verb is not on the
/// conservative non-mutating allowlist — connection lifecycle, applies, and
/// control must keep flowing. Because dispatch matches **exits before the
/// command table** (treaty ruling 5), a line that resolves to an exit from
/// the actor's position is refused even when it spells an allowlisted verb
/// (classic builds alias exits `l`, `look`, `ex`, ... — traversing one
/// moves the player, dirtying three objects). Unlisted-but-actually-
/// read-only commands are refused too (conservative by design); a mutating
/// command that slips through an allowlisted verb would still be queued for
/// the writer and retried in order, so the gate bounds divergence rather
/// than guarding correctness.
fn refuse_read_only(world: &World, msg: &Inbound) -> Option<(ObjId, String)> {
    let Inbound::Cmd { actor, line, .. } = msg else {
        return None;
    };
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // An actor inside an input context (mail compose, chargen) must never
    // be trapped there: their lines are arbitrary text, not verbs — the
    // allowlist would refuse the hard escape `~q` (which dispatch promises
    // ALWAYS pops) while letting allowlisted verbs through to be silently
    // captured as body text. Context feeds mutate only RAM and queue
    // ordered module ops — exactly the bounded divergence this gate's
    // contract accepts — so they pass wholesale.
    if world.input_depth(*actor) > 0 {
        return None;
    }
    // Speech prefixes never mutate the world (and dispatch rewrites them
    // before its exit match, so they cannot traverse an exit either).
    if matches!(t.chars().next(), Some('"' | ':' | ';' | '\\')) {
        return None;
    }
    let refusal = || {
        Some((
            *actor,
            "The database is currently unavailable; commands that change the world \
             are paused. Staff have been alerted -- please try again shortly."
                .to_string(),
        ))
    };
    // Exit match wins over the command table in dispatch, so it must win
    // over the allowlist here: movement mutates.
    if find_exit(world, *actor, t).is_some() {
        return refusal();
    }
    let verb = t
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    const ALLOWED: &[&str] = &[
        "look",
        "l",
        "who",
        "help",
        "think",
        "say",
        "pose",
        "semipose",
        "@emit",
        "examine",
        "ex",
        "inventory",
        "inv",
        "i",
        // Staff must be able to stop the server while the store is down.
        "@shutdown",
    ];
    if ALLOWED.contains(&verb.as_str()) {
        return None;
    }
    refusal()
}

/// Enter/leave read-only mode on a writer health transition, alerting
/// connected Wizards either way.
fn apply_store_status(
    world: &World,
    sessions: &SessionManager,
    health: &mut StoreHealth,
    healthy: bool,
    detail: &str,
) {
    if healthy {
        if health.read_only {
            health.read_only = false;
            tracing::info!("store recovered; leaving read-only mode");
            broadcast_wizards(
                world,
                sessions,
                "GAME: store writes recovered; the world is writable again.",
            );
        }
    } else if !health.read_only {
        health.read_only = true;
        tracing::error!(detail, "store write failure; entering read-only mode");
        broadcast_wizards(
            world,
            sessions,
            &format!(
                "GAME: store write failure ({detail}); mutating commands are \
                 paused until the store recovers."
            ),
        );
    }
}

/// Send a Sys line to every connected Wizard-or-better.
fn broadcast_wizards(world: &World, sessions: &SessionManager, text: &str) {
    let ev = Arc::new(OutputEvent::Sys(RichText::tag("system", text)));
    for actor in world.connected() {
        if world.get(actor).is_some_and(|o| o.role() >= Role::Wizard) {
            sessions.send_to_char(char_key(actor), Arc::clone(&ev));
        }
    }
}

/// What became of one [`flush_dirty`] call. The distinction between
/// [`Clean`](FlushOutcome::Clean) and
/// [`WriterGone`](FlushOutcome::WriterGone) matters to ack-gated callers: a
/// clean flush has nothing to persist, while a gone writer means a real
/// batch was **dropped** and must never be confirmed as durable.
enum FlushOutcome {
    /// Nothing was dirty; no batch was produced.
    Clean,
    /// A batch was handed to the writer. Carries the receipt receiver when
    /// the batch was durable.
    Queued(Option<oneshot::Receiver<Result<u64, String>>>),
    /// The writer's queue is closed (teardown); the batch was dropped.
    WriterGone,
}

/// Snapshot the world's dirty set into one batch and hand it to the writer,
/// bumping `serial` when a batch is actually produced.
///
/// Before batching, any dirty object marked `GOING` (an `@destroy`ed husk)
/// is [`sweep`](World::sweep)t to a tombstone so the **same batch** carries
/// its store delete — without this, destroyed objects would linger in RAM
/// and in the store forever (nothing else in the server calls `sweep`).
async fn flush_dirty(
    engine: &mut Engine,
    writer: &mpsc::Sender<StoreJob>,
    serial: &mut u64,
    durable: bool,
    health: &mut StoreHealth,
) -> FlushOutcome {
    let mut dirty = engine.world.take_dirty();
    // Module persistence ops (comsys/mail/jobs writes queued via
    // Ctx::persist) ride the SAME batch as the step's object mutations —
    // that is the write-through atomicity contract of the module API.
    let module_ops = engine.take_module_ops();
    if dirty.is_empty() && module_ops.is_empty() {
        return FlushOutcome::Clean;
    }
    // `destroy` dirties everything it marks GOING, and every flush sweeps,
    // so scanning the dirty set finds every husk — never the whole world.
    if dirty.iter().any(|&id| {
        engine
            .world
            .get(id)
            .is_some_and(|o| o.flags.contains(Flags::GOING))
    }) {
        engine.world.sweep();
        // Swept ids were re-marked dirty (as deletions); fold them in.
        dirty.append(&mut engine.world.take_dirty());
    }
    *serial += 1;
    let mut batch = persist::batch_from_dirty(&engine.world, dirty, *serial, durable);
    batch.module_ops = module_ops;
    if batch.is_empty() {
        return FlushOutcome::Clean;
    }
    let (notify, receipt) = if durable {
        let (tx, rx) = oneshot::channel();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };
    if writer
        .send(StoreJob::Write { batch, notify })
        .await
        .is_err()
    {
        tracing::error!(
            serial = *serial,
            "store writer gone; batch dropped (teardown)"
        );
        return FlushOutcome::WriterGone;
    }
    health.last_write_unix = Some(unix_now_s());
    FlushOutcome::Queued(receipt)
}

/// Handle the **local** staff ops (`@boot`, `@ps`, `@stats`) inline in the
/// world loop: they need the `Engine` (scheduler, world, loop-lag mirror)
/// and the session manager, and never touch the store. Output rides the
/// normal outbox; the caller delivers it right after.
fn run_local_staff_ops(
    engine: &mut Engine,
    sessions: &Arc<SessionManager>,
    health: &StoreHealth,
    boot_unix: i64,
    ops: Vec<StaffOp>,
    out: &mut Outbox,
    fx: &mut Effects,
) {
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::Boot {
                target,
                ref message,
            } => {
                tracing::info!(%actor, %target, message, "@boot requested");
                let name = engine
                    .world
                    .get(target)
                    .map_or_else(|| target.to_string(), |o| o.name.clone());
                // Close the whole session: every connection handle comes
                // back so each ConnDown(Boot) can be fed to the engine
                // (announce + world-side disconnect on the last one).
                // The handler's Disconnect event was already delivered, so
                // the target saw the message; dropping the handles closes
                // the transports once their queues drain.
                let mut booted = 0u32;
                if let Some(sess) = sessions.session_of_char(char_key(target)) {
                    for handle in sessions.close_session(sess) {
                        booted += 1;
                        engine.step(
                            Inbound::ConnDown {
                                conn: handle.id.0,
                                actor: target,
                                reason: DisconnectKind::Boot,
                            },
                            out,
                            fx,
                        );
                    }
                }
                if booted == 0 {
                    // No live connections (linkdead, or a test world with
                    // no session layer attachment): force the world-side
                    // disconnect so WHO and the room agree.
                    engine.step(Inbound::SessionGone { actor: target }, out, fx);
                    out.info(
                        actor,
                        format!("@boot: {name} had no live connections; session cleared."),
                    );
                } else {
                    let plural = if booted == 1 {
                        "connection"
                    } else {
                        "connections"
                    };
                    out.info(actor, format!("@boot: {name} booted ({booted} {plural})."));
                }
            }
            StaffOpKind::Ps => {
                tracing::info!(%actor, "@ps requested");
                out.info(actor, render_ps(engine, unix_now_ms()));
            }
            StaffOpKind::Stats => {
                tracing::info!(%actor, "@stats requested");
                out.info(actor, render_stats(engine, health, boot_unix, unix_now_s()));
            }
            // Store ops never reach here (see the partition in `run`).
            _ => {}
        }
    }
}

/// Truncate a command line for `@ps` display.
fn preview(line: &str) -> String {
    const MAX: usize = 40;
    if line.chars().count() <= MAX {
        line.to_string()
    } else {
        let cut: String = line.chars().take(MAX).collect();
        format!("{cut}...")
    }
}

/// Human form of a tick cadence: `coarse 300s`, `fast 250ms`, `scene`.
fn cadence_text(c: TickCadence) -> String {
    match c {
        TickCadence::Coarse(d) => format!("coarse {}s", d.as_secs().max(1)),
        TickCadence::Fast(d) => format!("fast {}ms", d.as_millis().max(1)),
        TickCadence::Scene => "scene".to_string(),
    }
}

/// Render the `@ps` scheduler report: pending `@wait`s (actor, seconds
/// remaining, command preview), module ticks (tag, cadence, next fire),
/// scene ticks, and totals. Pure over the engine — unit-testable.
#[must_use]
pub fn render_ps(engine: &Engine, now_ms: i64) -> String {
    let entries = engine.sched.pending_entries();
    let mut next_fire: std::collections::HashMap<(String, String), i64> =
        std::collections::HashMap::new();
    let mut waits = Vec::new();
    let mut scenes = Vec::new();
    let mut control = 0usize;
    for &(due, kind) in &entries {
        let remain_s = (due - now_ms).max(0) / 1000;
        match kind {
            ScheduledKind::WaitCmd { actor, line } => {
                let name = engine
                    .world
                    .get(*actor)
                    .map_or_else(|| actor.to_string(), |o| o.name.clone());
                waits.push(format!("  {name}  in {remain_s}s: {}", preview(line)));
            }
            ScheduledKind::ModuleTick { module, tag } => {
                next_fire.insert((module.to_string(), tag.to_string()), remain_s);
            }
            ScheduledKind::SceneTick { module, tag, scene } => {
                scenes.push(format!("  {module}:{tag}  {scene}  next in {remain_s}s"));
            }
            ScheduledKind::Control(_) => control += 1,
        }
    }
    let mut ticks = Vec::new();
    for t in engine.registry.module_ticks() {
        let when = next_fire
            .get(&(t.module.to_string(), t.tag.to_string()))
            .map_or_else(String::new, |s| format!("  next in {s}s"));
        ticks.push(format!(
            "  {}:{}  {}{when}",
            t.module,
            t.tag,
            cadence_text(t.cadence)
        ));
    }
    let mut text = String::from("Scheduler:");
    text.push_str(&format!("\nWaits: {}", waits.len()));
    for w in &waits {
        text.push('\n');
        text.push_str(w);
    }
    text.push_str(&format!("\nModule ticks: {}", ticks.len()));
    for t in &ticks {
        text.push('\n');
        text.push_str(t);
    }
    text.push_str(&format!("\nScene ticks: {}", scenes.len()));
    for s in &scenes {
        text.push('\n');
        text.push_str(s);
    }
    if control > 0 {
        text.push_str(&format!("\nControl timers: {control}"));
    }
    text.push_str(&format!("\nTotal scheduled entries: {}", entries.len()));
    text
}

/// Render the wizard `@stats` report: uptime, object counts by kind,
/// connected count, loop lag (last/max), store health (read-only mode +
/// last-write age), and db size. Pure over its inputs — unit-testable.
#[must_use]
pub fn render_stats(
    engine: &Engine,
    health: &StoreHealth,
    boot_unix: i64,
    now_unix: i64,
) -> String {
    let w = &engine.world;
    let (mut rooms, mut exits, mut things, mut players, mut npcs, mut going) =
        (0usize, 0, 0, 0, 0, 0usize);
    for o in w.iter() {
        if o.flags.contains(Flags::GOING) {
            going += 1;
            continue;
        }
        match o.kind {
            ObjKind::Room => rooms += 1,
            ObjKind::Exit => exits += 1,
            ObjKind::Thing => things += 1,
            ObjKind::Player => players += 1,
            ObjKind::Npc => npcs += 1,
        }
    }
    let total = rooms + exits + things + players + npcs;
    let connected = w.connected().count();
    let (lag_last, lag_max) = engine.loop_lag_ms();
    let uptime = (now_unix - boot_unix).max(0);
    let store = if health.read_only {
        "READ-ONLY (writes failing; mutations paused)".to_string()
    } else {
        "healthy".to_string()
    };
    let write_age = match health.last_write_unix {
        Some(at) => format!("{}s ago", (now_unix - at).max(0)),
        None => "no writes yet this boot".to_string(),
    };
    let mut text = String::from("Server statistics:");
    text.push_str(&format!(
        "\nUptime: {} (since unix {boot_unix})",
        human_duration(uptime)
    ));
    text.push_str(&format!(
        "\nObjects: {total} ({rooms} rooms, {exits} exits, {things} things, \
         {players} players, {npcs} NPCs)"
    ));
    if going > 0 {
        text.push_str(&format!("\nPending destruction: {going}"));
    }
    text.push_str(&format!("\nConnected: {connected}"));
    text.push_str(&format!("\nLoop lag: last {lag_last}ms, max {lag_max}ms"));
    text.push_str(&format!("\nStore: {store}; last write {write_age}"));
    text.push_str(&format!(
        "\nDb size: {total} objects (next id #{})",
        w.next_id()
    ));
    text
}

/// `3661` → `"1h 1m 1s"`.
fn human_duration(mut s: i64) -> String {
    let h = s / 3600;
    s %= 3600;
    let m = s / 60;
    s %= 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Hand off the staff ops the step's command handlers queued. Ban ops
/// (set/clear/list) go to the **sequential** [`staff_ops_loop`] so two
/// staff commands issued in order can never race in the store; `@dump`
/// rides the store-writer queue (ordered behind pending batches);
/// `@shutdown` returns `true` — the caller (the world loop) then steps
/// `ControlMsg::Shutdown` and exits. Every op is audit-logged with the
/// requesting actor.
async fn run_staff_ops(
    _deps: &ServerDeps,
    sessions: &Arc<SessionManager>,
    writer: &mpsc::Sender<StoreJob>,
    staff_ops: &mpsc::Sender<StaffOp>,
    ops: Vec<StaffOp>,
) -> bool {
    let mut shutdown = false;
    for op in ops {
        let actor = op.actor;
        match op.kind {
            StaffOpKind::SetBan {
                ref ip,
                until_unix,
                ref reason,
            } => {
                tracing::info!(%actor, %ip, ?until_unix, reason, "@siteban requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(sessions, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::ClearBan { ref ip } => {
                tracing::info!(%actor, %ip, "@siteban/clear requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(
                        sessions,
                        actor,
                        "@siteban/clear failed: staff executor gone.",
                    );
                }
            }
            StaffOpKind::ListBans => {
                tracing::info!(%actor, "@siteban list requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(sessions, actor, "@siteban failed: staff executor gone.");
                }
            }
            StaffOpKind::NewPassword { target, .. } => {
                // Audit: actor + target only — the op's password never
                // reaches a log (Secret redacts even derived Debug).
                tracing::info!(%actor, %target, "@newpassword requested");
                if staff_ops.send(op).await.is_err() {
                    send_sys(sessions, actor, "@newpassword failed: staff executor gone.");
                }
            }
            StaffOpKind::Dump => {
                tracing::info!(%actor, "@dump requested");
                let writer = writer.clone();
                let sessions = Arc::clone(sessions);
                tokio::spawn(async move {
                    let (done, done_rx) = oneshot::channel();
                    if writer.send(StoreJob::Bgsave { done }).await.is_err() {
                        send_sys(&sessions, actor, "@dump failed: store writer gone.");
                        return;
                    }
                    let text = match done_rx.await {
                        Ok(Ok(())) => "@dump: snapshot acknowledged by the store.".to_string(),
                        Ok(Err(e)) => format!("@dump failed: {e}"),
                        Err(_) => "@dump failed: store writer gone.".to_string(),
                    };
                    send_sys(&sessions, actor, &text);
                });
            }
            StaffOpKind::Shutdown => {
                tracing::info!(%actor, "@shutdown requested");
                shutdown = true;
            }
            // Local ops are partitioned out before this call.
            StaffOpKind::Boot { .. } | StaffOpKind::Ps | StaffOpKind::Stats => {
                debug_assert!(false, "local staff op reached the store executor path");
            }
        }
    }
    shutdown
}

/// Execute one `@newpassword`: argon2-hash on the blocking pool, find the
/// owning account by scanning the account docs (characters are few; the
/// store has no reverse index yet), update the stored hash, then revoke
/// every web token of that account (logout-everywhere) **and boot the
/// account's live web connections** — web transports are token-
/// authenticated, so they die with the tokens (SSH sessions are session-
/// authenticated and stay; boot them with `@boot` if needed). Returns the
/// confirmation text for the requesting wizard; delivers the target's
/// notification itself.
async fn run_newpassword(
    store: &dyn ObjectStore,
    sessions: &SessionManager,
    world: &mpsc::WeakSender<WorldMsg>,
    target: ObjId,
    name: &str,
    by: &str,
    password: &str,
) -> String {
    let hash = {
        let password = password.to_string();
        match tokio::task::spawn_blocking(move || crate::accounts::hash_password(&password)).await {
            Ok(Ok(h)) => h,
            Ok(Err(e)) => return format!("@newpassword failed: {e}."),
            Err(e) => return format!("@newpassword failed: hashing task failed: {e}."),
        }
    };
    let account = match store.list_accounts().await {
        Ok(accounts) => accounts
            .into_iter()
            .find(|a| a.characters.iter().any(|c| c.id == target)),
        Err(e) => return format!("@newpassword failed: {e}"),
    };
    let Some(account) = account else {
        return format!("@newpassword: {name} is not linked to any account.");
    };
    match store.set_account_password(&account.name, &hash).await {
        Ok(true) => {}
        Ok(false) => {
            return format!(
                "@newpassword failed: account {} vanished mid-update.",
                account.name
            );
        }
        Err(e) => return format!("@newpassword failed: {e}"),
    }
    // Logout-everywhere: every web session token of the account dies with
    // the old password. (Live SSH connections are session-authenticated
    // and stay; boot them separately with @boot if needed.)
    let revoked = match store.tokens_del_account(&account.name).await {
        Ok(n) => n,
        Err(e) => {
            // The password DID change; report the partial failure honestly.
            return format!(
                "@newpassword: password for {name} changed, but revoking web \
                 sessions failed: {e}"
            );
        }
    };
    tracing::info!(%target, account = %account.name, revoked, "@newpassword applied");
    // Notify BEFORE booting, so a web-connected target actually sees why
    // their session is about to close.
    send_sys(
        sessions,
        target,
        &format!("GAME: Your password has been changed by {by}."),
    );
    // The tokens are dead; so are the live connections they opened. Every
    // structured (web) connection on any of the account's characters is
    // booted — cleanly, through the world (announce + bookkeeping) when it
    // is still up.
    let world = world.upgrade().map(|tx| WorldHandle { tx });
    let mut booted = 0u32;
    for ch in &account.characters {
        let Some(sess) = sessions.session_of_char(char_key(ch.id)) else {
            continue;
        };
        for handle in sessions.conns_of_session(sess) {
            if !handle.is_structured() {
                continue;
            }
            if let Some(world) = &world {
                world
                    .inbound(Inbound::ConnDown {
                        conn: handle.id.0,
                        actor: ch.id,
                        reason: DisconnectKind::Boot,
                    })
                    .await;
                world.barrier().await;
            }
            send_event_to_conn(
                sessions,
                handle.id,
                OutputEvent::Disconnect {
                    reason: RichText::tag(
                        "system",
                        format!("Your password has been changed by {by}; please log in again."),
                    ),
                },
            );
            sessions.detach(handle.id, DetachReason::Booted);
            booted += 1;
        }
    }
    format!(
        "@newpassword: Password for {name} (account {}) changed; \
         {revoked} web session token{} revoked, {booted} live web \
         connection{} booted.",
        account.name,
        if revoked == 1 { "" } else { "s" },
        if booted == 1 { "" } else { "s" },
    )
}

/// The sequential staff-op executor: one task, one queue, ops applied to
/// the store (and mirrored into the [`IpGuard`]) strictly in the order
/// staff issued them — a `@siteban/clear` can never lose to the `@siteban`
/// it follows. Confirmations are delivered to the requesting actor after
/// the store acknowledged. Exits when the world loop drops its sender.
async fn staff_ops_loop(
    store: Arc<dyn ObjectStore>,
    guard: Arc<IpGuard>,
    sessions: Arc<SessionManager>,
    world: mpsc::WeakSender<WorldMsg>,
    mut rx: mpsc::Receiver<StaffOp>,
) {
    while let Some(op) = rx.recv().await {
        let actor = op.actor;
        let text = match op.kind {
            StaffOpKind::SetBan {
                ip,
                until_unix,
                reason,
            } => match store.set_ban(&ip.to_string(), until_unix, &reason).await {
                Ok(()) => {
                    guard.set_siteban(ip, until_unix);
                    match until_unix {
                        None => format!("@siteban: {ip} banned permanently."),
                        Some(until) => format!("@siteban: {ip} banned until unix {until}."),
                    }
                }
                Err(e) => format!("@siteban failed: {e}"),
            },
            StaffOpKind::ClearBan { ip } => match store.clear_ban(&ip.to_string()).await {
                Ok(removed) => {
                    guard.clear_siteban(ip);
                    if removed {
                        format!("@siteban: ban on {ip} lifted.")
                    } else {
                        format!("@siteban: no ban on {ip}.")
                    }
                }
                Err(e) => format!("@siteban/clear failed: {e}"),
            },
            StaffOpKind::NewPassword {
                target,
                name,
                by,
                password,
            } => run_newpassword(&*store, &sessions, &world, target, &name, &by, &password.0).await,
            StaffOpKind::ListBans => match store.get_bans().await {
                Ok(bans) if bans.is_empty() => "No active sitebans.".to_string(),
                Ok(mut bans) => {
                    bans.sort_by(|a, b| a.ip.cmp(&b.ip));
                    let mut text = String::from("Active sitebans:");
                    for b in bans {
                        let until = b
                            .until_unix
                            .map_or_else(|| "permanent".to_string(), |u| format!("until unix {u}"));
                        text.push_str(&format!("\n  {}  {}  {}", b.ip, until, b.reason));
                    }
                    text
                }
                Err(e) => format!("@siteban list failed: {e}"),
            },
            // Dump/Shutdown and the local ops never enter this queue (see
            // run_staff_ops / run_local_staff_ops).
            StaffOpKind::Dump
            | StaffOpKind::Shutdown
            | StaffOpKind::Boot { .. }
            | StaffOpKind::Ps
            | StaffOpKind::Stats => continue,
        };
        send_sys(&sessions, actor, &text);
    }
    tracing::debug!("staff op executor drained and exiting");
}

fn send_sys(sessions: &SessionManager, actor: ObjId, text: &str) {
    sessions.send_to_char(
        char_key(actor),
        Arc::new(OutputEvent::Sys(RichText::tag("system", text))),
    );
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

/// Create a Player in the start room (Limbo). Players own themselves.
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

/// One fully resolved send target (world-side audience resolution already
/// applied).
enum SendTarget {
    /// Every connection of one character.
    Char(CharKey),
    /// One specific connection.
    Conn(SessConnId),
    /// Everyone connected.
    All,
}

/// Resolve every queued audience against **current** world state into
/// per-character (or per-connection) sends. The world half of treaty
/// ruling 2 lives here: `Room` audiences become the room's *connected
/// actors*; the session manager later takes it from character to
/// connections. Split from [`transmit`] so a durable-gated step can
/// resolve now and deliver after the store ack.
fn resolve_sends(world: &World, out: &mut Outbox) -> Vec<(SendTarget, Arc<OutputEvent>)> {
    let mut sends = Vec::new();
    for (audience, event) in out.take() {
        let ev = Arc::new(event);
        match audience {
            Audience::Actor(actor) => {
                sends.push((SendTarget::Char(char_key(actor)), ev));
            }
            Audience::Conn(conn) => {
                sends.push((SendTarget::Conn(SessConnId(conn)), ev));
            }
            Audience::Room { room, exclude } => {
                // `@ignore` room filtering happens HERE — the one place a
                // Room audience resolves to listeners (treaty ruling 2's
                // world half). An attributed narrative line (say/pose/
                // semipose/@emit) from a speaker a listener has muted is
                // skipped for that listener only; staff speakers are never
                // filtered (mush_core::is_ignoring owns the policy).
                let speaker = narrative_actor(&ev);
                for &occupant in world.contents_of(room) {
                    if exclude.contains(&occupant) || !world.is_connected(occupant) {
                        continue;
                    }
                    if let Some(speaker) = speaker
                        && is_ignoring(world, occupant, speaker)
                    {
                        continue;
                    }
                    sends.push((SendTarget::Char(char_key(occupant)), Arc::clone(&ev)));
                }
            }
            Audience::All => sends.push((SendTarget::All, ev)),
        }
    }
    sends
}

/// Hand resolved sends to the session manager. Returns the connections
/// whose buffer overflowed (the event was dropped for them); the caller
/// disconnects them as slow clients.
fn transmit(
    sessions: &SessionManager,
    sends: Vec<(SendTarget, Arc<OutputEvent>)>,
) -> Vec<SessConnId> {
    let mut slow = Vec::new();
    for (target, ev) in sends {
        match target {
            SendTarget::Char(ch) => {
                let report = sessions.send_to_char(ch, ev);
                if report.delivered == 0 && report.dropped.is_empty() {
                    // Link-dead actor (or a race with a disconnect): the
                    // event is lost. Resume buffers are an M2+ concern
                    // (PLAN: mush-session "resume buffers"); make the loss
                    // observable at least.
                    tracing::debug!(
                        char = ch.0,
                        "output to actor with no live connections dropped"
                    );
                }
                slow.extend(report.dropped);
            }
            SendTarget::Conn(conn) => {
                // Pre-auth / connection-addressed events. An unknown conn is
                // fine (it may have raced a disconnect).
                let _ = sessions.send_to_conn(conn, ev);
            }
            SendTarget::All => {
                let report = sessions.broadcast_all(ev);
                slow.extend(report.dropped);
            }
        }
    }
    slow.sort_unstable();
    slow.dedup();
    slow
}

/// Resolve and transmit in one pass (the ordinary, un-gated path).
fn deliver(world: &World, sessions: &SessionManager, out: &mut Outbox) -> Vec<SessConnId> {
    transmit(sessions, resolve_sends(world, out))
}

/// Spawn one detached task per drained [`Effect`]; each completion re-enters
/// the world as a typed [`WorldCommand`] via [`Inbound::Apply`].
///
/// Each in-flight effect holds a strong sender (upgraded from the loop's
/// weak one), so the world task outlives every pending completion; once the
/// last handle *and* the last effect are gone, the channel closes and the
/// task ends.
fn spawn_effects(fx: &mut Effects, tx: &mpsc::WeakSender<WorldMsg>, llm: Option<&Arc<LlmService>>) {
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
            // M7 LLM re-entry seam (mush-core `Effect::ModuleAsync`, treaty
            // ruling 3): a module snapshotted context synchronously and
            // deferred its async work; run it OFF the world loop against the
            // service handle mushd holds, then re-enter as the typed
            // `WorldCommand::ModuleAsyncDone`. Routing is by module name —
            // mush-core never interprets the payload.
            Effect::ModuleAsync {
                module,
                actor,
                correlation,
                payload,
            } => {
                let Some(tx) = tx.upgrade() else {
                    tracing::debug!(%module, correlation, "world channel closing; module-async dropped");
                    continue;
                };
                // worldgen `@gen`: run the LLM room generator off-loop and
                // re-enter with the serialized GenOutcome. When the service is
                // absent (repl / a test rig with no LLM) the effect is dropped
                // — but the real server always constructs one, even when
                // `[llm].enabled = false` (its kill switch is engaged, so the
                // generator re-enters a "disabled" outcome instead). The
                // `.as_str()` borrow ends with the condition, so `module` is
                // free to move into the completion below.
                if module.as_str() == mush_worldgen::NS {
                    let Some(llm) = llm.cloned() else {
                        tracing::warn!(
                            correlation,
                            "worldgen Effect::ModuleAsync dropped: no LlmService on this world task"
                        );
                        continue;
                    };
                    tokio::spawn(async move {
                        let result = mush_worldgen::run_worldgen_async(&llm, &payload).await;
                        let done = WorldCommand::ModuleAsyncDone {
                            module,
                            actor,
                            correlation,
                            payload: result,
                        };
                        if tx
                            .send(WorldMsg::Inbound(Inbound::Apply(done)))
                            .await
                            .is_err()
                        {
                            tracing::debug!(
                                correlation,
                                "worldgen completion after shutdown dropped"
                            );
                        }
                    });
                } else {
                    tracing::warn!(
                        %module,
                        correlation,
                        "Effect::ModuleAsync for an unrouted module dropped"
                    );
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::module::{ContextOutcome, InputContext};
    use mush_core::{Ctx, GOD};
    use mush_proto::{InputMode, PromptSpec};

    fn cmd(line: &str) -> Inbound {
        Inbound::Cmd {
            conn: 1,
            actor: GOD,
            line: line.to_string(),
        }
    }

    /// The conservative allowlist really is a pin: module verbs — comsys
    /// aliases, `@mail`, `+jobs` — mutate module state (seq bumps, boxes,
    /// cursors) and MUST stay refused while read-only. If a future
    /// allowlist edit lets one through, this fails.
    #[test]
    fn read_only_refuses_module_verbs() {
        let world = World::new_minimal();
        for line in [
            "pub Hello there",
            "@mail/quick Nick=hi/there",
            "@mail",
            "addcom pub=Public",
            "comlist",
            "+request T=text",
            "+job/approve 1",
            "@channel/recall pub",
        ] {
            assert!(
                refuse_read_only(&world, &cmd(line)).is_some(),
                "{line:?} must be refused while read-only"
            );
        }
        for line in ["look", "WHO", "say hi", "\"hi", ":waves", "help mail"] {
            assert!(
                refuse_read_only(&world, &cmd(line)).is_none(),
                "{line:?} must pass while read-only"
            );
        }
    }

    struct Sink;

    impl InputContext for Sink {
        fn prompt(&self, _w: &World) -> PromptSpec {
            PromptSpec {
                mode: InputMode::Normal,
                text: None,
            }
        }

        fn feed(&mut self, _ctx: &mut Ctx<'_>, _line: &str) -> ContextOutcome {
            ContextOutcome::Consumed
        }
    }

    /// A player inside an input context (mail compose) must never be
    /// trapped by read-only mode: every line — the hard escape `~q`, the
    /// send terminator `--`, and arbitrary body text that happens to spell
    /// a refused verb — passes the gate and feeds the context.
    #[test]
    fn read_only_passes_input_context_lines() {
        let mut world = World::new_minimal();
        world.push_input_context(GOD, None, Box::new(Sink));
        for line in ["~q", "--", "body text", "@create orb", "pub Hello", "l"] {
            assert!(
                refuse_read_only(&world, &cmd(line)).is_none(),
                "{line:?} must pass while composing"
            );
        }
        // Popped: the gate applies again.
        world.pop_input_context(GOD);
        assert!(refuse_read_only(&world, &cmd("@create orb")).is_some());
    }
}
