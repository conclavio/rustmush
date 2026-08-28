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
//!    order); durability-critical flows (Player creation) carry a receipt
//!    oneshot and their confirmations wait for the ack;
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
    Outbox, Registry, Role, StepOutcome, World, WorldCommand, register_core_commands,
    register_engine_commands,
};
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
}

impl ServerDeps {
    /// Throwaway deps for tests and the compat [`spawn`]: fresh in-memory
    /// store, fresh guard, fresh (never-populated) staff queue.
    #[must_use]
    pub fn in_memory() -> ServerDeps {
        ServerDeps {
            store: Arc::new(InMemoryStore::new()),
            guard: Arc::new(IpGuard::new()),
            staff: Arc::new(StaffQueue::new()),
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
    tokio::spawn(staff_ops_loop(
        Arc::clone(&deps.store),
        Arc::clone(&deps.guard),
        Arc::clone(&sessions),
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
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    let mut serial: u64 = 0;
    let mut read_only = false;
    loop {
        let next_due = engine.sched.next_due();
        let wake = tokio::select! {
            m = rx.recv() => m.map_or(Wake::Closed, Wake::Msg),
            () = sleep_until_unix_ms(next_due), if next_due.is_some() => Wake::Timer,
        };
        let iteration_start = Instant::now();
        engine.set_now(unix_now_s());
        let outcome = match wake {
            Wake::Msg(WorldMsg::Inbound(msg)) => {
                if read_only && let Some(refused) = refuse_read_only(&engine.world, &msg) {
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
                if read_only {
                    // A creation now would mutate RAM behind a stuck writer
                    // queue and hang the requester on an ack that cannot
                    // come until the store recovers. Fail closed instead.
                    let _ = reply.send(Err(CreatePlayerErr::StoreUnavailable));
                } else {
                    match create_player(&mut engine, &name) {
                        Ok(id) => {
                            // Flush the creation batch (durable) and defer
                            // the reply until the store acks — the whole
                            // point of the gate.
                            match flush_dirty(&mut engine, &writer, &mut serial, true).await {
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
                apply_store_status(&engine.world, &sessions, &mut read_only, healthy, &detail);
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
        // Write-through: one batch per processed message that dirtied the
        // world (fire-and-forget; the writer preserves order and retries).
        let _ = flush_dirty(&mut engine, &writer, &mut serial, false).await;
        let staff_shutdown = run_staff_ops(&deps, &sessions, &writer, &staff_ops).await;
        // Loop-lag watchdog (PLAN risk 2): the single-writer loop must stay
        // fast; a slow iteration means a slow sync handler or writer-queue
        // backpressure.
        let lag = iteration_start.elapsed();
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
            spawn_effects(&mut fx, &tx);
            let _ = flush_dirty(&mut engine, &writer, &mut serial, false).await;
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
    read_only: &mut bool,
    healthy: bool,
    detail: &str,
) {
    if healthy {
        if *read_only {
            *read_only = false;
            tracing::info!("store recovered; leaving read-only mode");
            broadcast_wizards(
                world,
                sessions,
                "GAME: store writes recovered; the world is writable again.",
            );
        }
    } else if !*read_only {
        *read_only = true;
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
) -> FlushOutcome {
    let mut dirty = engine.world.take_dirty();
    if dirty.is_empty() {
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
    let batch = persist::batch_from_dirty(&engine.world, dirty, *serial, durable);
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
    FlushOutcome::Queued(receipt)
}

/// Hand off the staff ops the step's command handlers queued. Ban ops
/// (set/clear/list) go to the **sequential** [`staff_ops_loop`] so two
/// staff commands issued in order can never race in the store; `@dump`
/// rides the store-writer queue (ordered behind pending batches);
/// `@shutdown` returns `true` — the caller (the world loop) then steps
/// `ControlMsg::Shutdown` and exits. Every op is audit-logged with the
/// requesting actor.
async fn run_staff_ops(
    deps: &ServerDeps,
    sessions: &Arc<SessionManager>,
    writer: &mpsc::Sender<StoreJob>,
    staff_ops: &mpsc::Sender<StaffOp>,
) -> bool {
    let mut shutdown = false;
    for op in deps.staff.drain() {
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
        }
    }
    shutdown
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
            // Dump and Shutdown never enter this queue (see run_staff_ops).
            StaffOpKind::Dump | StaffOpKind::Shutdown => continue,
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
