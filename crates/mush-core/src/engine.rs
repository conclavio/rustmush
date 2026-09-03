//! The [`Engine`]: the synchronous heart the tokio world task drives.
//!
//! One loop iteration is: receive an [`Inbound`], call [`Engine::step`],
//! deliver the [`Outbox`], spawn everything in [`Effects`], sleep until
//! [`Scheduler::next_due`], and on wake call [`Engine::fire_due`]. The
//! engine itself is plain sync data — no I/O, no tokio — so every path
//! here is unit-testable with a manual clock.
//!
//! Async work follows treaty ruling 3, the **two-phase pattern**:
//!
//! 1. A handler snapshots what it needs, pushes an [`Effect`] (a
//!    future-free *description* of the async work) via
//!    [`Outbox::defer`], and returns immediately.
//! 2. The driving loop drains [`Effects`], spawns the real future, and
//!    when it completes sends [`Inbound::Apply`] with a typed
//!    [`WorldCommand`] carrying the correlation id. The engine's handler
//!    re-validates against **current** world state before applying.
//!
//! `@slowtest` is the M1 prototype of this seam: a fake 3-second "LLM
//! call" proving typed re-entry without ever blocking the game loop.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use mush_proto::{OutputEvent, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::action::{GuardRevalidation, GuardRevalidationError, GuardSet};
use crate::action_state::{
    ActionStateBootRecord, ActionStateError, ActionStateStore, ActionTxn, PreparedInstallation,
    PreparedTxn, StateFamilyHandle,
};
use crate::cmd::{
    ArgShape, CmdError, CommandSpec, Ctx, ParsedArgs, Registry, RegistryError, SwitchSet, dispatch,
};
use crate::commands::narrative_emit;
use crate::flags::Role;
use crate::module::{
    self, GameModule, ModError, ModuleBootData, ModuleOp, SceneId, TickCadence, WorldEvent,
};
use crate::obj::{GOD, ObjId};
use crate::output::{ConnId, DeliveryClass, Outbox, name_span};
use crate::sched::{ScheduledKind, Scheduler, TimerId};
use crate::world::World;

/// Cap on `@slowtest` delays (seconds) — it is a debug tool, not a timer.
const SLOWTEST_MAX_SECONDS: u64 = 60;

/// Cap on `@wait` delays: one year, keeping `due_ms` far from overflow.
const WAIT_MAX_SECONDS: u64 = 365 * 24 * 60 * 60;

/// Cap on *pending* `@wait` entries per actor (classic MUSH queue limit) —
/// without it any player could grow the scheduler heap without bound.
const WAIT_QUEUE_MAX: u32 = 100;

/// Why a connection went down — drives the engine's announcement and
/// whether the actor fully disconnects at once.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisconnectKind {
    /// Player-initiated clean close (`QUIT`): on the last connection the
    /// actor fully disconnects immediately ("has disconnected.").
    Quit,
    /// Server-initiated clean close (`@boot`, defensive cleanup): same
    /// immediate full disconnect as [`DisconnectKind::Quit`].
    Boot,
    /// The transport dropped unexpectedly: on the last connection the actor
    /// goes link-dead and stays connected until [`Inbound::SessionGone`].
    Lost,
}

impl DisconnectKind {
    /// Clean closes end the session at once; `Lost` leaves it link-dead.
    #[must_use]
    pub fn is_clean(self) -> bool {
        matches!(self, DisconnectKind::Quit | DisconnectKind::Boot)
    }
}

impl std::fmt::Display for DisconnectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DisconnectKind::Quit => "quit",
            DisconnectKind::Boot => "boot",
            DisconnectKind::Lost => "lost",
        })
    }
}

/// Operator/driver control messages (SIGTERM, `@shutdown`, `@dump`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlMsg {
    /// Announce and stop the world loop.
    Shutdown,
    /// Request a database dump. M1 has no store; the engine only announces
    /// (the M2 persistence layer will act on it in the driving loop).
    Dump,
}

/// Typed async re-entry (treaty ruling 3): every cross-crate async
/// completion comes back to the world as one of these, carried by
/// [`Inbound::Apply`]. Handlers **re-validate against current world
/// state** — the world may have changed while the async work ran.
///
/// Extensible by design: later milestones add arms here — reserved names
/// include `NpcAct { .. }` (M8 agent actions), `WorldgenDraftReady { .. }`
/// (M7 `@gen` drafts), and `AuthResult { .. }` (M2 login flows). Do not
/// repurpose those names.
///
/// The generic [`WorldCommand::ModuleAsyncDone`] arm (paired with
/// [`Effect::ModuleAsync`]) carries any of those as an **opaque module
/// payload** so a module (worldgen in M7, npc in M8) owns its own re-entry
/// shape without a new core arm per feature: the completion is routed to the
/// module's registered [`AsyncCompletion`](crate::module::AsyncCompletion)
/// handler, which decodes the payload and re-validates against current state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldCommand {
    /// Completion of an [`Effect::SlowTest`]: the fake LLM "answered".
    SlowTestDone {
        /// The actor who ran `@slowtest`.
        actor: ObjId,
        /// Correlation id the effect carried out.
        correlation: u64,
        /// The produced text, emitted to the actor.
        text: String,
    },
    /// Typed completion of an [`Effect::ModuleAsync`] (treaty ruling 3):
    /// routed in the world task to `module`'s registered
    /// [`AsyncCompletion`](crate::module::AsyncCompletion) handler. `payload`
    /// is module-owned JSON, carried as a `String` so the enum stays `Eq`.
    ModuleAsyncDone {
        /// The owning module (its [`GameModule::name`](crate::GameModule::name)).
        module: SmolStr,
        /// The triggering actor (the handler runs with this as `ctx.actor`).
        actor: ObjId,
        /// Correlation id echoed from the originating [`Effect::ModuleAsync`].
        correlation: u64,
        /// Opaque module-owned result payload (JSON as a string).
        payload: String,
    },
}

/// A future-free description of deferred async work. `mush-core` holds no
/// futures (no tokio here); the driving loop drains these from [`Effects`]
/// and spawns the real futures, each completing as an [`Inbound::Apply`].
///
/// Later milestones add variants (LLM calls, argon2 verification, store
/// queries); M1 has only the seam prototype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The fake 3-second "LLM call": sleep `seconds`, then re-enter with
    /// [`WorldCommand::SlowTestDone`] for `actor`/`correlation`.
    SlowTest {
        /// The actor to answer.
        actor: ObjId,
        /// Correlation id to echo back in the completion.
        correlation: u64,
        /// How long the fake call takes.
        seconds: u64,
    },
    /// Opaque async work owned by a module (the M7+ LLM re-entry seam). The
    /// driving loop routes it — by `module` name — to the runtime that holds
    /// the real service handle (e.g. `mushd`'s `LlmService` for worldgen),
    /// runs it off the world loop, and re-enters with
    /// [`WorldCommand::ModuleAsyncDone`]. `mush-core` never interprets
    /// `payload`; it is module-owned JSON carried as a `String` so the enum
    /// stays `Eq`.
    ModuleAsync {
        /// The owning module (its [`GameModule::name`](crate::GameModule::name)).
        module: SmolStr,
        /// The triggering actor, echoed into the completion.
        actor: ObjId,
        /// Correlation id echoed into the completion.
        correlation: u64,
        /// Opaque module-owned request payload (JSON as a string).
        payload: String,
    },
}

/// What a command handler deferred through [`Outbox::defer`], routed by
/// [`Engine::step`]: async work goes to [`Effects`], timers go to the
/// [`Scheduler`], module persistence ops go to the engine's module-op
/// queue (drained by the world task into the current `WriteBatch`).
#[derive(Clone, Debug, PartialEq)]
pub enum Deferred {
    /// Spawn this async work (via the [`Effects`] queue).
    Effect(Effect),
    /// Spawn async work under an explicit durability class. The legacy
    /// [`Effect`](Self::Effect) arm is exactly `OnCommit`.
    ClassifiedEffect {
        /// When the driver may release the effect.
        class: DeliveryClass,
        /// Future-free effect description.
        effect: Effect,
    },
    /// Schedule `kind` to fire `delay_ms` after the world's current time.
    ScheduleIn {
        /// Delay relative to [`World::now`], in milliseconds.
        delay_ms: i64,
        /// What fires.
        kind: ScheduledKind,
    },
    /// A module persistence op ([`Ctx::persist`](crate::cmd::Ctx::persist))
    /// — collected via [`Engine::take_module_ops`] into the per-command
    /// write batch.
    Persist(ModuleOp),
    /// Start (or re-arm with a new period) a per-scene module tick
    /// ([`Ctx::scene_tick_start`](crate::cmd::Ctx::scene_tick_start)).
    SceneTickStart {
        /// The owning module.
        module: SmolStr,
        /// The registered `Scene`-cadence tick tag.
        tag: SmolStr,
        /// The scene it is bound to.
        scene: SceneId,
        /// Period in milliseconds (>= 1).
        every_ms: i64,
    },
    /// Stop a running per-scene module tick.
    SceneTickStop {
        /// The owning module.
        module: SmolStr,
        /// The tick tag.
        tag: SmolStr,
        /// The scene.
        scene: SceneId,
    },
    /// Select the actor for the dirty step's durable-write failure diagnostic
    /// ([`Ctx::gate_on_durable_ack`](crate::cmd::Ctx::gate_on_durable_ack)).
    /// Receipt-gating itself is unconditional at the server boundary.
    DurableGate {
        /// The acting player whose confirmation is being gated.
        actor: ObjId,
        /// **Strict** (money) gating: on a dropped durable write the confirmation
        /// is SUPPRESSED, not delivered-with-a-warning. Invariant 9 — no posted
        /// confirmation may outrun the durable ack (E2 R-R1). A non-strict gate
        /// (mail/XP) still delivers the RAM effect with a "may not survive" warning.
        strict: bool,
    },
}

/// The queue of [`Effect`]s one loop iteration produced. The driving loop
/// drains it after every [`Engine::step`] / [`Engine::fire_due`] and spawns
/// the described futures.
#[derive(Debug, Default)]
pub struct Effects {
    queue: Vec<Effect>,
    classes: Vec<DeliveryClass>,
}

impl Effects {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Effects {
        Effects::default()
    }

    /// Queue one effect.
    pub fn push(&mut self, effect: Effect) {
        self.push_classified(DeliveryClass::OnCommit, effect);
    }

    /// Queue one effect with an explicit durability relationship.
    pub fn push_classified(&mut self, class: DeliveryClass, effect: Effect) {
        self.queue.push(effect);
        self.classes.push(class);
    }

    /// Drain the queue in push order, leaving it empty.
    pub fn drain(&mut self) -> Vec<Effect> {
        debug_assert_eq!(self.queue.len(), self.classes.len());
        self.classes.clear();
        std::mem::take(&mut self.queue)
    }

    /// Drain typed effects in push order.
    pub fn drain_classified(&mut self) -> Vec<(DeliveryClass, Effect)> {
        debug_assert_eq!(self.queue.len(), self.classes.len());
        let effects = std::mem::take(&mut self.queue);
        let classes = std::mem::take(&mut self.classes);
        classes.into_iter().zip(effects).collect()
    }

    /// The queued effects, in push order (for tests/inspection).
    #[must_use]
    pub fn queued(&self) -> &[Effect] {
        &self.queue
    }

    /// Durability classes parallel to [`queued`](Self::queued).
    #[must_use]
    pub fn classes(&self) -> &[DeliveryClass] {
        &self.classes
    }

    /// Number of queued effects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether nothing is queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Everything the world task can receive.
///
/// (`Eq` is deliberately not derived: [`Inbound::Query`] carries a
/// `serde_json::Value`, which is only `PartialEq`.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Inbound {
    /// One input line from a live connection.
    Cmd {
        /// The connection it arrived on.
        conn: ConnId,
        /// The actor it runs as.
        actor: ObjId,
        /// The raw line.
        line: String,
    },
    /// A structured panel-hydration request (`ClientCommand::Req`, M4): the
    /// web bridge maps each [`mush_proto::Query`] variant to a registered
    /// query-handler `name` plus JSON `args`. The engine dispatches to the
    /// registry's query handler and answers with
    /// [`OutputEvent::Reply`]`{ id, event }` addressed to `conn` only —
    /// queries are read-only and never stamp `last_input` (a background
    /// panel refresh must not reset WHO idle time).
    Query {
        /// The connection the reply goes to.
        conn: ConnId,
        /// The requesting actor (permissions are evaluated as them).
        actor: ObjId,
        /// Client-chosen correlation id, echoed in the `Reply`.
        id: u64,
        /// The registered query-handler name (e.g. `"who"`, `"mail.list"`,
        /// `"comsys.backlog"`).
        name: SmolStr,
        /// Handler-defined parameters.
        args: serde_json::Value,
    },
    /// A connection finished auth and attached to an actor.
    ConnUp {
        /// The new connection.
        conn: ConnId,
        /// The actor it attached to.
        actor: ObjId,
        /// Whether the client understands structured (JSON) events —
        /// recorded per connection; the session layer renders accordingly.
        caps_structured: bool,
        /// Whether this attach revived a link-dead session (the session
        /// layer's `AttachOutcome::Rejoined { from_linkdead }`); drives the
        /// classic "has reconnected." announce.
        from_linkdead: bool,
    },
    /// A connection dropped (network close, QUIT, boot). A clean
    /// [`DisconnectKind`] on the actor's last connection fully disconnects
    /// them at once; `Lost` leaves them link-dead until
    /// [`Inbound::SessionGone`].
    ConnDown {
        /// The connection that went away.
        conn: ConnId,
        /// The actor it was attached to.
        actor: ObjId,
        /// Why it went down.
        reason: DisconnectKind,
    },
    /// The session layer gave up on the actor entirely (linkdead grace
    /// expired). The actor fully disconnects — unless the engine currently
    /// tracks live connections for them, in which case the message is stale
    /// (a reconnect raced the reaper) and is ignored (treaty ruling 3:
    /// re-validate against current state).
    SessionGone {
        /// The actor whose session ended.
        actor: ObjId,
    },
    /// Typed completion of deferred async work (treaty ruling 3).
    Apply(WorldCommand),
    /// Operator/driver control.
    Control(ControlMsg),
}

/// What the driving loop should do after a step.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    /// Keep looping.
    Continue,
    /// Stop the world loop (shutdown was requested and announced).
    Shutdown,
}

/// Per-connection bookkeeping.
#[derive(Copy, Clone, Debug)]
struct ConnInfo {
    actor: ObjId,
    #[allow(dead_code)] // read by the session layer via caps negotiation later
    caps_structured: bool,
}

/// Bookkeeping for one running per-scene tick.
#[derive(Copy, Clone, Debug)]
struct SceneTickInfo {
    timer: TimerId,
    every_ms: i64,
}

/// A fully registered, mechanically frozen module set awaiting installation
/// into a [`World`].
///
/// Composition is deliberately separate from [`Engine`] construction so a
/// driver can finish and validate every module-owned registry before it loads
/// persisted world or module records. State slots are validated here but are
/// not installed until [`Engine::from_composition`] receives the world.
pub struct ModuleComposition {
    registry: Registry,
    modules: Vec<Box<dyn GameModule>>,
    state_inits: Vec<module::StateInit>,
}

/// One prepared action submission crossing the synchronous engine/driver
/// boundary. It is deliberately separate from legacy [`ModuleOp`] custody.
pub struct StagedAction {
    module_ops: Vec<ModuleOp>,
    installation: PreparedInstallation,
    guards: GuardSet,
}

impl fmt::Debug for StagedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedAction")
            .field("module_ops", &self.module_ops)
            .field("installation", &self.installation)
            .field("guards", &self.guards)
            .finish()
    }
}

impl StagedAction {
    /// Split the already-built persistence writes from the post-ack images.
    pub fn into_parts(self) -> (Vec<ModuleOp>, PreparedInstallation) {
        (self.module_ops, self.installation)
    }
}

/// Refusal to stage a prepared action beside legacy mutable work.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
pub enum StagePreparedActionError {
    #[error("another prepared action is already staged")]
    AlreadyStaged,
    #[error("prepared action cannot share a step with legacy object mutations")]
    DirtyWorld,
    #[error("prepared action cannot share a step with legacy module persistence")]
    LegacyModuleOps,
}

/// Final process-local refusal while moving a staged action into writer
/// custody. The staged typed images are always aborted before this is
/// returned.
#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq)]
pub enum StagedActionAdmissionError {
    #[error("{component} registry is unavailable at final action admission")]
    RegistryUnavailable { component: &'static str },
    #[error("final action guard evaluation failed: {0}")]
    Revalidation(#[from] GuardRevalidationError),
    #[error("prepared action guards became stale before commit admission")]
    Stale(GuardRevalidation),
}

impl ModuleComposition {
    /// The frozen command/module/mechanics registry.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The registered modules, in registration order.
    #[must_use]
    pub fn modules(&self) -> &[Box<dyn GameModule>] {
        &self.modules
    }
}

/// The sync world engine: world + command registry + scheduler + modules +
/// connection bookkeeping. The tokio loop in `mushd` owns exactly one of
/// these.
pub struct Engine {
    /// The game state.
    pub world: World,
    /// The command table (plus module registration tables).
    pub registry: Registry,
    /// Pending timers (`@wait`, module ticks).
    pub sched: Scheduler,
    /// The registered game modules (for [`Engine::modules_loaded`]).
    modules: Vec<Box<dyn GameModule>>,
    /// Engine-owned authoritative state for registered action families.
    action_state: ActionStateStore,
    /// At most one prepared action awaiting transfer to the driver.
    staged_action: Option<StagedAction>,
    /// Module persistence ops drained from handlers, awaiting
    /// [`Engine::take_module_ops`] (the world task folds them into the
    /// current `WriteBatch`).
    module_ops: Vec<ModuleOp>,
    /// Running per-scene ticks by `(module, tag, scene)`.
    scene_ticks: HashMap<(SmolStr, SmolStr, SceneId), SceneTickInfo>,
    /// Set when the last drained step requested a durable-ack output gate
    /// ([`Deferred::DurableGate`]); the driving loop takes it via
    /// [`Engine::take_durable_gate`]. The bool is strict (money) gating.
    durable_gate: Option<(ObjId, bool)>,
    /// Live connections by id.
    conns: HashMap<ConnId, ConnInfo>,
    /// Live connection count per actor (invariant: sums `conns` rows).
    conn_count: HashMap<ObjId, u32>,
    /// Pending `@wait` entries per actor (capped at [`WAIT_QUEUE_MAX`]).
    pending_waits: HashMap<ObjId, u32>,
    /// Most recent loop-iteration lag reported by the driver (ms).
    lag_last_ms: u64,
    /// Worst loop-iteration lag reported since boot (ms).
    lag_max_ms: u64,
}

impl Engine {
    /// Wrap a world and a fully registered command table. The caller
    /// registers commands first (typically
    /// [`register_core_commands`](crate::register_core_commands) followed by
    /// [`register_engine_commands`]).
    #[must_use]
    pub fn new(world: World, registry: Registry) -> Engine {
        let action_state = registry
            .state_families()
            .cloned()
            .map_or_else(ActionStateStore::unregistered, ActionStateStore::new);
        Engine {
            world,
            registry,
            sched: Scheduler::new(),
            modules: Vec::new(),
            action_state,
            staged_action: None,
            module_ops: Vec::new(),
            scene_ticks: HashMap::new(),
            durable_gate: None,
            conns: HashMap::new(),
            conn_count: HashMap::new(),
            pending_waits: HashMap::new(),
            lag_last_ms: 0,
            lag_max_ms: 0,
        }
    }

    /// Record one loop-iteration duration from the driving loop's watchdog
    /// (the M2 loop-lag metric). The engine only mirrors it so sync staff
    /// tooling (`@stats`) can report it.
    pub fn record_loop_lag(&mut self, lag_ms: u64) {
        self.lag_last_ms = lag_ms;
        self.lag_max_ms = self.lag_max_ms.max(lag_ms);
    }

    /// The `(last, max)` loop-iteration lag in milliseconds recorded via
    /// [`Engine::record_loop_lag`].
    #[must_use]
    pub fn loop_lag_ms(&self) -> (u64, u64) {
        (self.lag_last_ms, self.lag_max_ms)
    }

    /// Compose and validate game modules without a world.
    ///
    /// Each module's [`GameModule::register`] runs against the one registry,
    /// then the mechanical registries are frozen. State-slot ownership is
    /// validated but the slots are retained in the returned composition and
    /// cannot affect a world until [`Engine::from_composition`] is called.
    pub fn compose_modules(
        mut registry: Registry,
        mut modules: Vec<Box<dyn GameModule>>,
    ) -> Result<ModuleComposition, ModError> {
        // A mechanics owner is the canonical form of a module name. Preflight
        // the complete roster before calling even one `register`: otherwise
        // e.g. `Foo` and `foo` could become the same OwnerId while appearing
        // to be distinct module names, silently sharing definition ownership.
        // Defer the non-canonical spelling error until after the collision pass
        // so a roster containing both spellings gets the better diagnostic.
        let mut seen_owners: HashMap<crate::mechanics::OwnerId, &'static str> = HashMap::new();
        let mut noncanonical_name: Option<(&'static str, String)> = None;
        for m in &modules {
            let name = m.name();
            let owner = crate::mechanics::OwnerId::new(name).map_err(|error| {
                ModError::msg(format!(
                    "module name {name:?} is not a valid mechanics owner id: {error}"
                ))
            })?;
            if let Some(registered) = seen_owners.insert(owner.clone(), name) {
                return Err(ModError::msg(format!(
                    "module names {registered:?} and {name:?} resolve to the same canonical mechanics owner {owner:?}"
                )));
            }
            if owner.as_str() != name && noncanonical_name.is_none() {
                noncanonical_name = Some((name, owner.to_string()));
            }
        }
        if let Some((name, canonical)) = noncanonical_name {
            return Err(ModError::msg(format!(
                "module name {name:?} is not canonical; use {canonical:?}"
            )));
        }
        let module_owners: BTreeSet<_> = seen_owners.keys().cloned().collect();
        for m in &mut modules {
            let name = m.name();
            let mut registrar = module::Registrar::for_composition(&mut registry, name);
            m.register(&mut registrar)?;
        }
        // Definition/action/bridge composition is complete before any world
        // state exists or persisted record can be interpreted.
        registry.freeze_mechanics(&module_owners)?;

        // State-slot conflict check: a second module declaring the same
        // TypeId would silently wipe the first module's slot (and with it
        // e.g. every registered jobs payload handler). A module
        // re-declaring its OWN slot keeps the last value.
        let state_inits = registry.take_state_inits();
        let mut slot_owners: HashMap<std::any::TypeId, SmolStr> = HashMap::new();
        for init in &state_inits {
            if let Some(owner) = slot_owners.get(&init.type_id()) {
                if owner != init.module() {
                    return Err(ModError::msg(format!(
                        "module {:?}: state slot already declared by module {owner:?} \
                         (one slot type, one owner)",
                        init.module()
                    )));
                }
            } else {
                slot_owners.insert(init.type_id(), init.module().clone());
            }
        }

        Ok(ModuleComposition {
            registry,
            modules,
            state_inits,
        })
    }

    /// Install a validated [`ModuleComposition`] into `world` and arm its
    /// periodic module ticks.
    ///
    /// Every `Coarse`/`Fast` tick is first due one period from
    /// [`World::now`]. Call [`World::set_now`] before this method when the
    /// world's clock should start anywhere other than its default.
    #[must_use]
    pub fn from_composition(world: World, composition: ModuleComposition) -> Engine {
        let ModuleComposition {
            registry,
            modules,
            state_inits,
        } = composition;
        let mut engine = Engine::new(world, registry);
        engine.modules = modules;
        for init in state_inits {
            engine.world.module_state_apply(init);
        }
        engine.arm_registered_periodic_ticks();
        engine
    }

    /// [`Engine::new`] plus game modules. This compatibility wrapper composes
    /// and freezes the module registries, then immediately installs the
    /// resulting state slots into `world`.
    ///
    /// After the driver has loaded each module's persisted data, it calls
    /// [`Engine::modules_loaded`].
    pub fn with_modules(
        world: World,
        registry: Registry,
        modules: Vec<Box<dyn GameModule>>,
    ) -> Result<Engine, ModError> {
        let composition = Self::compose_modules(registry, modules)?;
        Ok(Self::from_composition(world, composition))
    }

    /// Arm every registered non-scene module tick relative to the world's
    /// current clock.
    fn arm_registered_periodic_ticks(&mut self) {
        let base_ms = self.world.now().saturating_mul(1000);
        let periodic: Vec<(SmolStr, SmolStr, i64)> = self
            .registry
            .module_ticks()
            .iter()
            .filter_map(|t| match t.cadence {
                TickCadence::Coarse(d) | TickCadence::Fast(d) => Some((
                    t.module.clone(),
                    t.tag.clone(),
                    crate::module::duration_ms(d),
                )),
                TickCadence::Scene => None,
            })
            .collect();
        for (module, tag, period) in periodic {
            self.sched.insert(
                base_ms.saturating_add(period),
                ScheduledKind::ModuleTick { module, tag },
            );
        }
    }

    /// Deliver each module its persisted boot data (loaded by the driver
    /// via the store read API, out-of-band) after the world is up. Each
    /// module's [`GameModule::on_world_loaded`] runs with a full
    /// [`ModuleCtx`](crate::module::ModuleCtx); anything it defers
    /// (persistence ops, timers) is routed exactly like a command's.
    pub fn modules_loaded(
        &mut self,
        mut boot: HashMap<SmolStr, ModuleBootData>,
        out: &mut Outbox,
        fx: &mut Effects,
    ) -> Result<(), ModError> {
        let mut modules = std::mem::take(&mut self.modules);
        let mut result = Ok(());
        for m in &mut modules {
            let data = boot.remove(m.name()).unwrap_or_default();
            let module_name = SmolStr::new(m.name());
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module_name),
            };
            if let Err(e) = m.on_world_loaded(&mut ctx, data) {
                result = Err(e);
                break;
            }
        }
        self.modules = modules;
        self.drain_deferred(out, fx);
        result
    }

    /// Drain the module persistence ops collected since the last call, in
    /// emission order. The world task appends them to the same
    /// `WriteBatch` as the step's dirty objects (write-through, one
    /// atomic commit).
    pub fn take_module_ops(&mut self) -> Vec<ModuleOp> {
        std::mem::take(&mut self.module_ops)
    }

    /// Read-only access to engine-owned action state.
    #[must_use]
    pub const fn action_state(&self) -> &ActionStateStore {
        &self.action_state
    }

    /// Atomically install persisted typed state after module composition but
    /// before any module receives its ordinary boot callback.
    pub fn hydrate_action_state(
        &mut self,
        records: impl IntoIterator<Item = ActionStateBootRecord>,
    ) -> Result<(), ActionStateError> {
        self.action_state.hydrate(records)
    }

    /// Begin a handle-scoped transaction, primarily for bootstrapping and
    /// contract tests. Registered action providers use the stricter
    /// provider-owner dispatch path.
    pub fn action_transaction<'a, K, V>(
        &'a mut self,
        handle: &StateFamilyHandle<K, V>,
    ) -> Result<ActionTxn<'a>, ActionStateError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.action_state.transaction(handle)
    }

    /// Resolve and run the exact registered provider against the private
    /// owner-scoped action store. Providers receive no `World`, `Ctx`, raw
    /// module state, or persistence surface.
    pub fn prepare_registered_action(
        &mut self,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
    ) -> Result<PreparedTxn, crate::action::ActionProviderPrepareError> {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_invocation(action, invocation, &mut self.action_state)
    }

    /// Move a logical action's persistence and install images into the one
    /// driver-facing prepared lane.
    ///
    /// A prepared action is never mixed with the legacy dirty-object/module
    /// snapshot path. On refusal its store reservation is explicitly aborted,
    /// leaving byte-identical live action state.
    pub fn stage_prepared_action(
        &mut self,
        prepared: PreparedTxn,
        guards: GuardSet,
    ) -> Result<(), StagePreparedActionError> {
        if self.staged_action.is_some() {
            let (_, installation) = prepared.into_parts();
            self.action_state.abort(installation);
            return Err(StagePreparedActionError::AlreadyStaged);
        }
        let (module_ops, installation) = prepared.into_parts();
        if self.world.has_dirty() {
            self.action_state.abort(installation);
            return Err(StagePreparedActionError::DirtyWorld);
        }
        if !self.module_ops.is_empty() {
            self.action_state.abort(installation);
            return Err(StagePreparedActionError::LegacyModuleOps);
        }
        self.staged_action = Some(StagedAction {
            module_ops,
            installation,
            guards,
        });
        Ok(())
    }

    /// Whether one prepared action awaits transfer to the world task.
    #[must_use]
    pub const fn has_staged_action(&self) -> bool {
        self.staged_action.is_some()
    }

    /// Whether legacy world/module persistence work currently exists outside
    /// the prepared-action lane. The driver rechecks this immediately before
    /// submission so work introduced after staging cannot create a mixed
    /// authoritative batch.
    #[must_use]
    pub fn has_legacy_persistence_work(&self) -> bool {
        self.world.has_dirty() || !self.module_ops.is_empty()
    }

    /// Revalidate every server-created guard against current immutable truth
    /// and transfer the action to the driver in the same synchronous turn.
    /// Stale or unevaluable work is aborted before an error is returned.
    pub fn take_staged_action(
        &mut self,
    ) -> Result<Option<StagedAction>, StagedActionAdmissionError> {
        let Some(staged) = self.staged_action.as_ref() else {
            return Ok(None);
        };
        let revalidation = (|| {
            let revisions = self.registry.revision_validators().ok_or(
                StagedActionAdmissionError::RegistryUnavailable {
                    component: "revision-validator",
                },
            )?;
            let authority = self.registry.authority_guard_validators().ok_or(
                StagedActionAdmissionError::RegistryUnavailable {
                    component: "authority-validator",
                },
            )?;
            let definitions = self.registry.definitions().ok_or(
                StagedActionAdmissionError::RegistryUnavailable {
                    component: "definition",
                },
            )?;
            staged
                .guards
                .revalidate(&self.world, revisions, authority, definitions)
                .map_err(StagedActionAdmissionError::Revalidation)
        })();
        match revalidation {
            Ok(result) if result.is_current() => Ok(self.staged_action.take()),
            Ok(result) => {
                self.abort_staged_action();
                Err(StagedActionAdmissionError::Stale(result))
            }
            Err(error) => {
                self.abort_staged_action();
                Err(error)
            }
        }
    }

    fn abort_staged_action(&mut self) {
        if let Some(staged) = self.staged_action.take() {
            let (_, installation) = staged.into_parts();
            self.action_state.abort(installation);
        }
    }

    /// Install typed post-images after the exact applied receipt.
    pub fn install_prepared_action(&mut self, installation: PreparedInstallation) {
        self.action_state.install(installation);
    }

    /// Discard typed post-images after an indeterminate/refused receipt.
    pub fn abort_prepared_action(&mut self, installation: PreparedInstallation) {
        self.action_state.abort(installation);
    }

    /// The registered game modules, in registration order. The driver uses
    /// this between [`Engine::with_modules`] and [`Engine::modules_loaded`]
    /// to ask each module which persisted streams its boot data needs
    /// ([`GameModule::boot_stream_requests`]).
    #[must_use]
    pub fn modules(&self) -> &[Box<dyn GameModule>] {
        &self.modules
    }

    /// Emit one world event to subscribed observers from **outside** a
    /// command dispatch (driver-side mutations such as the account
    /// service's player creation), then route anything the observers
    /// deferred — so their persistence ops land in this engine's module-op
    /// queue and ride the caller's next `WriteBatch`, exactly as if the
    /// event had been emitted during a step.
    pub fn emit_world_event(&mut self, ev: &WorldEvent, out: &mut Outbox, fx: &mut Effects) {
        module::emit_world_event(&self.registry, &mut self.world, out, ev);
        self.drain_deferred(out, fx);
    }

    /// The module persistence ops queued so far (tests/inspection).
    #[must_use]
    pub fn module_ops(&self) -> &[ModuleOp] {
        &self.module_ops
    }

    /// Feed wall time in (unix seconds) — forwarded to [`World::set_now`].
    /// The driving loop calls this before each step; core logic never reads
    /// the system clock.
    pub fn set_now(&mut self, unix_seconds: i64) {
        self.world.set_now(unix_seconds);
    }

    /// The actor a live connection is attached to.
    #[must_use]
    pub fn actor_of(&self, conn: ConnId) -> Option<ObjId> {
        self.conns.get(&conn).map(|c| c.actor)
    }

    /// Number of live connections attached to an actor.
    #[must_use]
    pub fn connections_of(&self, actor: ObjId) -> u32 {
        self.conn_count.get(&actor).copied().unwrap_or(0)
    }

    /// Handle one inbound message. Output lands in `out`; deferred async
    /// work lands in `fx` (and timers in [`Engine::sched`]). Returns
    /// whether the loop should keep running.
    pub fn step(&mut self, msg: Inbound, out: &mut Outbox, fx: &mut Effects) -> StepOutcome {
        let outcome = match msg {
            Inbound::Cmd {
                conn: _,
                actor,
                line,
            } => {
                if self.world.get(actor).is_some() {
                    self.world.session_input(actor);
                    dispatch(&self.registry, &mut self.world, actor, &line, out);
                } else {
                    tracing::warn!(%actor, "Cmd for missing actor dropped");
                }
                StepOutcome::Continue
            }
            Inbound::Query {
                conn,
                actor,
                id,
                name,
                args,
            } => {
                self.handle_query(conn, actor, id, &name, &args, out);
                StepOutcome::Continue
            }
            Inbound::ConnUp {
                conn,
                actor,
                caps_structured,
                from_linkdead,
            } => {
                self.conn_up(conn, actor, caps_structured, from_linkdead, out);
                StepOutcome::Continue
            }
            Inbound::ConnDown {
                conn,
                actor,
                reason,
            } => {
                self.conn_down(conn, actor, reason, out);
                StepOutcome::Continue
            }
            Inbound::SessionGone { actor } => {
                self.session_gone(actor, out);
                StepOutcome::Continue
            }
            Inbound::Apply(cmd) => {
                self.apply(cmd, out);
                StepOutcome::Continue
            }
            Inbound::Control(ControlMsg::Shutdown) => {
                out.to_all(OutputEvent::Sys(RichText::tag(
                    "system",
                    "GAME: Shutdown in progress.",
                )));
                StepOutcome::Shutdown
            }
            Inbound::Control(ControlMsg::Dump) => {
                out.to_all(OutputEvent::Sys(RichText::tag(
                    "system",
                    "GAME: Dumping database.",
                )));
                StepOutcome::Continue
            }
        };
        self.drain_deferred(out, fx);
        outcome
    }

    /// Fire one bounded slice of scheduled entries due at `now_ms` (unix
    /// milliseconds):
    /// `WaitCmd` dispatches its line as its actor; module ticks run their
    /// registered [`TickHandler`](crate::module::TickHandler) and re-arm;
    /// scene ticks additionally stop when their scene is gone. The driving
    /// loop calls this when the [`Scheduler::next_due`] sleep elapses. If a
    /// synchronized deadline exceeds
    /// [`crate::sched::MAX_SCHEDULED_FIRES_PER_STEP`], the untouched remainder
    /// stays due and is drained by subsequent loop iterations.
    pub fn fire_due(&mut self, now_ms: i64, out: &mut Outbox, fx: &mut Effects) {
        for kind in self.sched.pop_due(now_ms) {
            match kind {
                ScheduledKind::WaitCmd { actor, line } => {
                    self.dec_pending_waits(actor);
                    // Re-validate: the actor may be gone by the time the
                    // timer fires.
                    if self.world.get(actor).is_some() {
                        dispatch(&self.registry, &mut self.world, actor, &line, out);
                    } else {
                        tracing::debug!(%actor, "WaitCmd for missing actor dropped");
                    }
                }
                ScheduledKind::ModuleTick { module, tag } => {
                    self.fire_module_tick(module, tag, now_ms, out);
                }
                ScheduledKind::SceneTick { module, tag, scene } => {
                    self.fire_scene_tick(module, tag, scene, now_ms, out);
                }
                ScheduledKind::Control(name) => {
                    tracing::debug!(%name, "control timer with no consumer (M1)");
                }
            }
        }
        self.drain_deferred(out, fx);
    }

    /// One periodic (`Coarse`/`Fast`) module tick: run the handler, re-arm
    /// one period from the fire time.
    fn fire_module_tick(&mut self, module: SmolStr, tag: SmolStr, now_ms: i64, out: &mut Outbox) {
        let Some((handler, cadence)) = self
            .registry
            .tick_reg(&module, &tag)
            .map(|t| (Arc::clone(&t.handler), t.cadence))
        else {
            tracing::debug!(%module, %tag, "module tick with no registration dropped");
            return;
        };
        let (TickCadence::Coarse(d) | TickCadence::Fast(d)) = cadence else {
            tracing::debug!(%module, %tag, "Scene-cadence tick fired as periodic; dropped");
            return;
        };
        {
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module.clone()),
            };
            handler.tick(&mut ctx, &tag, None);
        }
        let period = crate::module::duration_ms(d);
        self.sched.insert(
            now_ms.saturating_add(period),
            ScheduledKind::ModuleTick { module, tag },
        );
    }

    /// One per-scene tick: runs only while the scene is alive and the tick
    /// has not been stopped; re-arms itself afterwards.
    fn fire_scene_tick(
        &mut self,
        module: SmolStr,
        tag: SmolStr,
        scene: SceneId,
        now_ms: i64,
        out: &mut Outbox,
    ) {
        let key = (module.clone(), tag.clone(), scene);
        let Some(info) = self.scene_ticks.get(&key).copied() else {
            return; // stopped
        };
        if self.world.scene(scene).is_none() {
            // The scene ended: the tick dies with it.
            self.scene_ticks.remove(&key);
            return;
        }
        let Some(handler) = self
            .registry
            .tick_reg(&module, &tag)
            .map(|t| Arc::clone(&t.handler))
        else {
            self.scene_ticks.remove(&key);
            return;
        };
        {
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module.clone()),
            };
            handler.tick(&mut ctx, &tag, Some(scene));
        }
        let timer = self.sched.insert(
            now_ms.saturating_add(info.every_ms),
            ScheduledKind::SceneTick { module, tag, scene },
        );
        self.scene_ticks.insert(
            key,
            SceneTickInfo {
                timer,
                every_ms: info.every_ms,
            },
        );
    }

    // ------------------------------------------------------------------
    // Inbound handlers
    // ------------------------------------------------------------------

    fn conn_up(
        &mut self,
        conn: ConnId,
        actor: ObjId,
        caps_structured: bool,
        from_linkdead: bool,
        out: &mut Outbox,
    ) {
        self.conn_up_inner(conn, actor, caps_structured, from_linkdead, true, out);
    }

    /// The shared body of [`conn_up`](Self::conn_up) and
    /// [`reattach`](Self::reattach): rebuild ALL per-connection bookkeeping —
    /// the `conns` map, the live-connection count, `connected`/`session_start`,
    /// and the input-context prompt re-sync. When `announce` is set (the normal
    /// connect path) it additionally announces "has connected/reconnected" to
    /// the room and, on a genuine world-side connect transition, fires the
    /// connect hook and the [`WorldEvent::Connected`]. The silent re-attach path
    /// (M16c) passes `announce = false` so a mass restart re-attach rebuilds
    /// state WITHOUT spamming every room or re-firing Connected for the whole
    /// game (invariant R2).
    ///
    /// Returns whether this was the actor's FIRST live connection (`n == 1`).
    fn conn_up_inner(
        &mut self,
        conn: ConnId,
        actor: ObjId,
        caps_structured: bool,
        from_linkdead: bool,
        announce: bool,
        out: &mut Outbox,
    ) -> bool {
        if self.world.get(actor).is_none() {
            tracing::warn!(%actor, conn, "ConnUp for missing actor ignored");
            return false;
        }
        let prev = self.conns.insert(
            conn,
            ConnInfo {
                actor,
                caps_structured,
            },
        );
        if let Some(p) = prev {
            if p.actor == actor {
                // Duplicate ConnUp for a tracked connection: caps updated,
                // nothing else to do.
                return false;
            }
            // A conn id re-used for a different actor should not happen;
            // recover by releasing the old attachment.
            tracing::warn!(conn, old = %p.actor, new = %actor, "conn id re-attached");
            self.dec_conn_count(p.actor);
            let old = p.actor;
            self.world.set_live_conns(old, self.connections_of(old));
        }
        let n = self.conn_count.entry(actor).or_insert(0);
        *n += 1;
        let n = *n;
        self.world.set_live_conns(actor, n);
        // Reconnecting into a live input context (mail compose survives a
        // linkdead drop) must re-sync the connection's prompt and echo
        // mode, or the player types "look" into their mail body with a
        // normal-mode editor. Conn-addressed: only the NEW connection needs
        // the state pushed.
        if self.world.input_depth(actor) > 0 {
            let spec = crate::module::restored_prompt(&self.world, actor);
            out.to_conn(conn, OutputEvent::Prompt(spec));
        }
        if n == 1 {
            // set_connected/session_start are idempotent, so reviving a
            // link-dead session keeps the original connect time.
            let was_connected = self.world.is_connected(actor);
            self.world.set_connected(actor, true);
            self.world.session_start(actor);
            if announce {
                let suffix = if from_linkdead {
                    " has reconnected."
                } else {
                    " has connected."
                };
                announce_room(&self.world, actor, suffix, out);
                // Hooks/observers fire on the world-side transition only — a
                // link-dead revival never "connected" twice.
                if !was_connected {
                    module::fire_conn_hook(&self.registry, &mut self.world, out, actor, true);
                    module::emit_world_event(
                        &self.registry,
                        &mut self.world,
                        out,
                        &WorldEvent::Connected { who: actor },
                    );
                }
            }
        } else if announce {
            announce_room(&self.world, actor, " has reconnected.", out);
        }
        n == 1
    }

    /// Silent re-attach of a held connection after a server restart (M16c).
    ///
    /// Rebuilds the connection's world-side bookkeeping EXACTLY like
    /// [`conn_up`](Self::conn_up) — the `conns` map, the live-connection count,
    /// `connected`/`session_start`, the input-context prompt re-sync — but
    /// WITHOUT the room "has connected/reconnected" announce, the connect hook,
    /// or the [`WorldEvent::Connected`]: a mass re-attach of every held session
    /// must not spam rooms or re-fire Connected for the whole game (R2). The
    /// connect/session state, in-RAM after a boot, is reconstructed from the
    /// gateway's held sessions; world objects + durable module state already
    /// reloaded from the store on boot (R1).
    ///
    /// On the actor's FIRST re-attached connection — and only when it is not
    /// sitting in an input context (post-restart contexts are empty, so this
    /// normally proceeds) — it invokes the read-only room renderer directly,
    /// pushing the reconnected player a current view from the reloaded world
    /// (D2). It must not dispatch the text `look`: exits resolve before commands,
    /// so an exit carrying that alias could otherwise turn transport reattach
    /// into movement during an integrity freeze. The render addresses the actor,
    /// so a multi-connected session renders once and the gateway fans it out to
    /// every held connection.
    pub fn reattach(
        &mut self,
        conn: ConnId,
        actor: ObjId,
        caps_structured: bool,
        out: &mut Outbox,
    ) {
        let first = self.conn_up_inner(conn, actor, caps_structured, false, false, out);
        if first
            && self.world.input_depth(actor) == 0
            && let Some(room) = self.world.get(actor).and_then(|record| record.location)
        {
            crate::commands::show_room(&self.registry, &self.world, actor, room, out);
        }
    }

    fn conn_down(&mut self, conn: ConnId, actor: ObjId, reason: DisconnectKind, out: &mut Outbox) {
        let Some(info) = self.conns.remove(&conn) else {
            tracing::debug!(conn, %actor, "ConnDown for untracked connection ignored");
            return;
        };
        debug_assert_eq!(info.actor, actor, "ConnDown actor mismatch");
        let actor = info.actor; // trust our own bookkeeping over the message
        let last = self.dec_conn_count(actor);
        self.world.set_live_conns(actor, self.connections_of(actor));
        tracing::info!(conn, %actor, %reason, last, "connection down");
        if last && self.world.is_connected(actor) {
            if reason.is_clean() {
                // Clean close of the last connection: full disconnect now.
                announce_room(&self.world, actor, " has disconnected.", out);
                self.world.set_connected(actor, false);
                self.world.session_end(actor);
                module::fire_conn_hook(&self.registry, &mut self.world, out, actor, false);
                module::emit_world_event(
                    &self.registry,
                    &mut self.world,
                    out,
                    &WorldEvent::Disconnected { who: actor },
                );
            } else {
                // The character stays in the room (and connected, for WHO)
                // — linkdead until the session layer sends SessionGone.
                announce_room(&self.world, actor, " has gone link-dead.", out);
            }
        }
    }

    fn session_gone(&mut self, actor: ObjId, out: &mut Outbox) {
        // Re-validate (treaty ruling 3): the reaper's SessionGone is not
        // ordered against a concurrent reconnect's ConnUp. If the engine
        // currently tracks live connections for the actor, the reconnect
        // won the race and this message is stale — applying it would leave
        // a live session world-side "disconnected" (deaf to its room).
        if self.connections_of(actor) > 0 {
            tracing::info!(%actor, "stale SessionGone ignored (live connections present)");
            return;
        }
        // Defensive: forget any connections still attached to the actor.
        self.conns.retain(|_, i| i.actor != actor);
        self.conn_count.remove(&actor);
        self.world.set_live_conns(actor, 0);
        if self.world.is_connected(actor) {
            announce_room(&self.world, actor, " has disconnected.", out);
            self.world.set_connected(actor, false);
            module::fire_conn_hook(&self.registry, &mut self.world, out, actor, false);
            module::emit_world_event(
                &self.registry,
                &mut self.world,
                out,
                &WorldEvent::Disconnected { who: actor },
            );
        }
        self.world.session_end(actor);
    }

    /// Answer one [`Inbound::Query`]: look up the registered
    /// [`QueryHandler`](crate::module::QueryHandler) by `name`, run it
    /// read-only as `actor`, and address the wrapped
    /// [`OutputEvent::Reply`] to the requesting connection only. Every
    /// outcome — success, handler refusal, unknown name — produces exactly
    /// one `Reply` carrying the request's `id`, so the client can always
    /// correlate. A query for a missing actor is dropped (the connection is
    /// mid-teardown; treaty ruling 3 re-validation).
    fn handle_query(
        &self,
        conn: ConnId,
        actor: ObjId,
        id: u64,
        name: &str,
        args: &serde_json::Value,
        out: &mut Outbox,
    ) {
        if self.world.get(actor).is_none() {
            tracing::warn!(%actor, name, "Query for missing actor dropped");
            return;
        }
        let event = match self.registry.query_handler(name) {
            Some((_module, handler)) => match handler.query(&self.world, actor, args) {
                Ok(event) => event,
                Err(err) => OutputEvent::Sys(err),
            },
            None => OutputEvent::Sys(RichText::tag("error", format!("Unknown query {name:?}."))),
        };
        out.best_effort_to_conn(
            conn,
            OutputEvent::Reply {
                id,
                event: Box::new(event),
            },
        );
    }

    fn apply(&mut self, cmd: WorldCommand, out: &mut Outbox) {
        match cmd {
            WorldCommand::SlowTestDone {
                actor,
                correlation,
                text,
            } => {
                // Re-validate (treaty ruling 3): the actor may have been
                // destroyed while the fake call ran.
                if self.world.get(actor).is_none() {
                    tracing::debug!(%actor, correlation, "SlowTestDone for missing actor dropped");
                    return;
                }
                tracing::debug!(%actor, correlation, "slowtest completed");
                out.info(actor, text);
            }
            WorldCommand::ModuleAsyncDone {
                module,
                actor,
                correlation,
                payload,
            } => {
                // Disjoint field borrows (world mut, registry shared), the
                // same shape emit_world_event uses. The handler re-validates
                // against current state (treaty ruling 3).
                module::apply_async_completion(
                    &self.registry,
                    &mut self.world,
                    out,
                    module,
                    actor,
                    correlation,
                    &payload,
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    /// Route everything handlers deferred: effects to `fx`, timers to the
    /// scheduler (due = [`World::now`] seconds + delay). `@wait` entries are
    /// capped per actor ([`WAIT_QUEUE_MAX`]) so no player can grow the heap
    /// without bound.
    fn drain_deferred(&mut self, out: &mut Outbox, fx: &mut Effects) {
        for d in out.take_deferred() {
            match d {
                Deferred::Effect(e) => fx.push(e),
                Deferred::ClassifiedEffect { class, effect } => {
                    fx.push_classified(class, effect);
                }
                Deferred::ScheduleIn { delay_ms, kind } => {
                    if let ScheduledKind::WaitCmd { actor, .. } = &kind {
                        let actor = *actor;
                        let n = self.pending_waits.entry(actor).or_insert(0);
                        if *n >= WAIT_QUEUE_MAX {
                            out.best_effort_error(actor, "Too many commands queued.");
                            continue;
                        }
                        *n += 1;
                    }
                    let due = self
                        .world
                        .now()
                        .saturating_mul(1000)
                        .saturating_add(delay_ms);
                    self.sched.insert(due, kind);
                }
                Deferred::Persist(op) => self.module_ops.push(op),
                Deferred::SceneTickStart {
                    module,
                    tag,
                    scene,
                    every_ms,
                } => {
                    let key = (module.clone(), tag.clone(), scene);
                    // Re-arming an already-running tick replaces its timer.
                    if let Some(prev) = self.scene_ticks.get(&key) {
                        self.sched.cancel(prev.timer);
                    }
                    let due = self
                        .world
                        .now()
                        .saturating_mul(1000)
                        .saturating_add(every_ms);
                    let timer = self
                        .sched
                        .insert(due, ScheduledKind::SceneTick { module, tag, scene });
                    self.scene_ticks
                        .insert(key, SceneTickInfo { timer, every_ms });
                }
                Deferred::SceneTickStop { module, tag, scene } => {
                    if let Some(info) = self.scene_ticks.remove(&(module, tag, scene)) {
                        self.sched.cancel(info.timer);
                    }
                }
                Deferred::DurableGate { actor, strict } => {
                    // First gate of the step sets the actor; strictness DOMINATES —
                    // if any gate in the step is strict, the whole step is strict
                    // (money-safe: a step that moves money is never downgraded).
                    let entry = self.durable_gate.get_or_insert((actor, false));
                    entry.1 |= strict;
                }
            }
        }
    }

    /// Take the durable-ack output gate the last step requested (if any):
    /// the driving loop flushes that step's batch `durable: true` and
    /// withholds its output until the receipt resolves. The bool is strict
    /// (money) gating — on a dropped write, suppress the confirmation entirely.
    pub fn take_durable_gate(&mut self) -> Option<(ObjId, bool)> {
        self.durable_gate.take()
    }

    /// Release one pending-`@wait` slot for an actor (its timer fired).
    fn dec_pending_waits(&mut self, actor: ObjId) {
        match self.pending_waits.get_mut(&actor) {
            Some(n) if *n > 1 => *n -= 1,
            Some(_) => {
                self.pending_waits.remove(&actor);
            }
            None => {}
        }
    }

    /// Decrement an actor's live-connection count; `true` if it reached 0.
    fn dec_conn_count(&mut self, actor: ObjId) -> bool {
        match self.conn_count.get_mut(&actor) {
            Some(n) if *n > 1 => {
                *n -= 1;
                false
            }
            Some(_) => {
                self.conn_count.remove(&actor);
                true
            }
            None => false,
        }
    }
}

/// Queue an unattributed narrative line "<Name><suffix>" to the actor's
/// room, excluding the actor (connect/disconnect/linkdead announcements).
///
/// The line reveals `actor`, so it carries them as the M11 perception subject
/// ([`Outbox::to_room_from`]): a concealed actor's connect/disconnect echo is
/// gated from observers who cannot perceive them, exactly like their movement
/// echoes. Free pass with no `Perceive` hook registered.
fn announce_room(w: &World, actor: ObjId, suffix: &str, out: &mut Outbox) {
    if let Some(loc) = w.get(actor).and_then(|o| o.location) {
        let text = RichText::new().span(name_span(w, actor)).text(suffix);
        out.to_room_from(loc, vec![actor], actor, narrative_emit(text));
    }
}

// ----------------------------------------------------------------------
// Engine commands: @slowtest and @wait
// ----------------------------------------------------------------------

fn slowtest_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw();
    let seconds: u64 = if raw.is_empty() {
        3
    } else {
        raw.parse().map_err(|_| {
            CmdError::msg(format!(
                "Usage: @slowtest [<seconds>] (1-{SLOWTEST_MAX_SECONDS})."
            ))
        })?
    };
    if !(1..=SLOWTEST_MAX_SECONDS).contains(&seconds) {
        return Err(CmdError::msg(format!(
            "Usage: @slowtest [<seconds>] (1-{SLOWTEST_MAX_SECONDS})."
        )));
    }
    let correlation = ctx.w.next_correlation_id();
    ctx.out.defer(Deferred::Effect(Effect::SlowTest {
        actor: ctx.actor,
        correlation,
        seconds,
    }));
    ctx.out.info(ctx.actor, "Thinking...");
    Ok(())
}

fn wait_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const USAGE: &str = "Usage: @wait <seconds>=<command>.";
    let seconds: u64 = ctx.args.lhs().parse().map_err(|_| CmdError::msg(USAGE))?;
    if seconds > WAIT_MAX_SECONDS {
        return Err(CmdError::msg("That's too long to wait."));
    }
    let line = ctx.args.rhs().map(str::trim).unwrap_or_default();
    if line.is_empty() {
        return Err(CmdError::msg(USAGE));
    }
    // i64 is safe: seconds <= WAIT_MAX_SECONDS.
    let delay_ms = i64::try_from(seconds).expect("bounded above") * 1000;
    ctx.out.defer(Deferred::ScheduleIn {
        delay_ms,
        kind: ScheduledKind::WaitCmd {
            actor: ctx.actor,
            line: line.to_string(),
        },
    });
    // Classic MUX @wait queues silently.
    Ok(())
}

/// Register the engine's commands (`@slowtest`, `@wait`). Called at boot
/// after [`register_core_commands`](crate::register_core_commands).
pub fn register_engine_commands(reg: &mut Registry) -> Result<(), RegistryError> {
    reg.register(
        CommandSpec {
            name: "@slowtest".into(),
            shape: ArgShape::Raw,
            min_role: Role::Wizard,
            help: "@slowtest [<seconds>]  --  Wizard debug tool: start a fake slow \
                   async call (default 3 seconds, max 60) that answers you when it \
                   completes. You see \"Thinking...\" immediately; the game never \
                   blocks while it runs. This prototypes the LLM re-entry path."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(slowtest_cmd),
    )?;
    reg.register(
        CommandSpec {
            name: "@wait".into(),
            shape: ArgShape::EqPair,
            help: "@wait <seconds>=<command>  --  Queue <command> to run as you \
                   after <seconds> seconds (whole seconds, up to one year). The \
                   command is dispatched exactly as if you typed it then. Queues \
                   silently, classic-style."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(wait_cmd),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::register_core_commands;
    use crate::flags::Flags;
    use crate::obj::{GOD, LIMBO, ObjKind};
    use crate::output::{Audience, DeliveryOutcome};
    use mush_proto::NarrativeKind;

    /// Engine with core + engine commands, God connected via conn 1, and a
    /// player Nick (not yet connected) in Limbo.
    fn setup() -> (Engine, ObjId) {
        let mut world = World::new_minimal();
        world.set_now(1_000);
        let nick = world
            .create(ObjKind::Player, "Nick", GOD, Some(LIMBO))
            .unwrap();
        let mut registry = Registry::new();
        register_core_commands(&mut registry).unwrap();
        register_engine_commands(&mut registry).unwrap();
        let mut engine = Engine::new(world, registry);
        let mut out = Outbox::new();
        let mut fx = Effects::new();
        engine.step(
            Inbound::ConnUp {
                conn: 1,
                actor: GOD,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        (engine, nick)
    }

    fn texts(out: &Outbox) -> Vec<String> {
        out.events()
            .iter()
            .map(|(_, e)| match e {
                OutputEvent::Sys(t) => t.to_plain_text(),
                OutputEvent::Narrative { text, .. } => text.to_plain_text(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn module_composition_defers_state_install_until_world_is_supplied() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct DeferredState(u32);

        struct StateModule;
        impl GameModule for StateModule {
            fn name(&self) -> &'static str {
                "state_module"
            }

            fn register(&mut self, reg: &mut module::Registrar<'_>) -> Result<(), ModError> {
                reg.state(DeferredState(7));
                Ok(())
            }
        }

        let composition =
            Engine::compose_modules(Registry::new(), vec![Box::new(StateModule)]).unwrap();
        assert_eq!(composition.modules().len(), 1);
        assert_eq!(composition.state_inits.len(), 1);

        let engine = Engine::from_composition(World::new_minimal(), composition);
        assert_eq!(
            engine.world.module_state::<DeferredState>(),
            Some(&DeferredState(7))
        );
    }

    fn room_saw(out: &Outbox, room: ObjId, needle: &str) -> bool {
        out.events().iter().any(|(aud, e)| {
            matches!(aud, Audience::Room { room: r, .. } if *r == room)
                && matches!(e, OutputEvent::Narrative { text, .. } if text.to_plain_text().contains(needle))
        })
    }

    #[test]
    fn conn_up_announces_connect_then_reconnect() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has connected."));
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);
        assert_eq!(engine.actor_of(2), Some(nick));
        assert_eq!(engine.connections_of(nick), 1);

        // Second connection: reconnect announce, connect_time untouched.
        engine.set_now(1_050);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has reconnected."));
        assert_eq!(engine.connections_of(nick), 2);
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);

        // The announcement excludes the actor themselves.
        assert!(out.events().iter().all(
            |(aud, _)| matches!(aud, Audience::Room { exclude, .. } if exclude.contains(&nick))
        ));
    }

    #[test]
    fn reattach_is_silent_and_rebuilds_state_then_looks() {
        // M16c silent re-attach (R2): after a server restart the held
        // connections are re-attached WITHOUT a room "has connected/reconnected"
        // announce (and without re-firing Connected), but with the full
        // connection state rebuilt and a fresh look pushed to the player.
        let (mut engine, nick) = setup();
        // Exit matching precedes command lookup. This hostile-but-valid alias
        // proves reattach renders directly instead of dispatching a command.
        let trap = engine
            .world
            .create(ObjKind::Room, "Trap", GOD, None)
            .unwrap();
        let look_exit = engine
            .world
            .create(ObjKind::Exit, "look", GOD, Some(LIMBO))
            .unwrap();
        engine.world.link_exit(look_exit, Some(trap)).unwrap();
        let _ = engine.world.take_dirty();
        let (mut out, _fx) = (Outbox::new(), Effects::new());
        engine.reattach(2, nick, false, &mut out);

        // R2: no room connect/reconnect announce.
        assert!(
            !room_saw(&out, LIMBO, "has connected."),
            "silent re-attach must not announce a connect: {:?}",
            texts(&out)
        );
        assert!(
            !room_saw(&out, LIMBO, "has reconnected."),
            "silent re-attach must not announce a reconnect: {:?}",
            texts(&out)
        );
        // State rebuilt exactly like `conn_up`: connected, one live connection,
        // a session started, the conn mapped to the actor.
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.connections_of(nick), 1);
        assert!(engine.world.session_stat(nick).is_some());
        assert_eq!(engine.actor_of(2), Some(nick));
        assert_eq!(engine.world.get(nick).unwrap().location, Some(LIMBO));
        assert!(
            engine.world.take_dirty().is_empty(),
            "transport-only reattach must not dirty durable world state"
        );
        // The reconnected player is pushed a fresh room render (D2): the room
        // title reaches them even though nothing was announced to the room.
        assert!(
            texts(&out).iter().any(|t| t.contains("Limbo")),
            "re-attach must push a fresh look; got {:?}",
            texts(&out)
        );

        // A SECOND held connection of the same actor re-attaches just as
        // silently (no reconnect announce), and the live-connection count tracks
        // it — a multi-connected session rebuilds correctly.
        let (mut out2, _fx2) = (Outbox::new(), Effects::new());
        engine.reattach(3, nick, false, &mut out2);
        assert!(
            !room_saw(&out2, LIMBO, "has reconnected."),
            "second silent re-attach must stay silent: {:?}",
            texts(&out2)
        );
        assert_eq!(engine.connections_of(nick), 2);
    }

    #[test]
    fn conn_up_for_missing_actor_is_ignored() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 9,
                actor: ObjId(99),
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
        assert_eq!(engine.actor_of(9), None);
    }

    #[test]
    fn conn_down_announces_linkdead_only_on_last_connection() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        for conn in [2, 3] {
            engine.step(
                Inbound::ConnUp {
                    conn,
                    actor: nick,
                    caps_structured: false,
                    from_linkdead: false,
                },
                &mut out,
                &mut fx,
            );
        }
        // Dropping one of two connections: silent.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
        assert_eq!(engine.connections_of(nick), 1);

        // Dropping the last: link-dead announce, still connected for WHO.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 3,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has gone link-dead."));
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.connections_of(nick), 0);

        // A duplicate ConnDown is a no-op.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 3,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn session_gone_disconnects_and_announces() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // Link goes down (link-dead), then the grace expires: SessionGone.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(room_saw(&out, LIMBO, "Nick has disconnected."));
        assert!(!engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick), None);
        assert_eq!(engine.actor_of(2), None, "lingering conns are forgotten");

        // A second SessionGone announces nothing.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(out.is_empty());
    }

    /// The reaper's `SessionGone` is not ordered against a concurrent
    /// reconnect's `ConnUp`: when the ConnUp wins the race, the stale
    /// SessionGone must be ignored, not applied (it would leave a live
    /// session world-side "disconnected" — deaf to its room, absent from
    /// WHO, with no recovery).
    #[test]
    fn stale_session_gone_after_reconnect_is_ignored() {
        let (mut engine, nick) = setup();
        // Connect, lose the link (link-dead), reconnect out of linkdead.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // The reaper's SessionGone arrives late: ignored outright.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(Inbound::SessionGone { actor: nick }, &mut out, &mut fx);
        assert!(out.is_empty(), "stale SessionGone must not announce");
        assert!(engine.world.is_connected(nick));
        assert_eq!(engine.actor_of(3), Some(nick));
        assert_eq!(engine.connections_of(nick), 1);
        assert!(engine.world.session_stat(nick).is_some());
    }

    /// A clean last-connection ConnDown (QUIT) fully disconnects at once:
    /// "has disconnected.", connected flag cleared, session stats ended —
    /// no link-dead phase and no SessionGone needed.
    #[test]
    fn clean_conn_down_disconnects_immediately() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Quit,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has disconnected."));
        assert!(!room_saw(&out, LIMBO, "link-dead"));
        assert!(!engine.world.is_connected(nick));
        assert_eq!(engine.world.session_stat(nick), None);
        assert_eq!(engine.connections_of(nick), 0);
    }

    /// Reviving a link-dead session announces "has reconnected." (the
    /// driver forwards the session layer's `from_linkdead`), and keeps the
    /// original connect time.
    #[test]
    fn linkdead_revival_announces_reconnected() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.step(
            Inbound::ConnDown {
                conn: 2,
                actor: nick,
                reason: DisconnectKind::Lost,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_040);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 3,
                actor: nick,
                caps_structured: false,
                from_linkdead: true,
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "Nick has reconnected."));
        assert!(!room_saw(&out, LIMBO, "Nick has connected."));
        // Connect time survives the link-dead gap (session_start idempotent).
        assert_eq!(engine.world.session_stat(nick).unwrap().connect_time, 1_000);
    }

    #[test]
    fn cmd_dispatches_and_stamps_last_input() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_030);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "say hello".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(room_saw(&out, LIMBO, "hello"));
        assert_eq!(engine.world.session_stat(nick).unwrap().last_input, 1_030);

        // A Cmd for a missing actor neither panics nor emits.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 7,
                actor: ObjId(99),
                line: "say ghost".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn who_reports_real_idle_and_conn_seconds() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_010);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "think idling now".into(),
            },
            &mut out,
            &mut fx,
        );
        // 40s later, God runs WHO: Nick is idle 40s, connected 50s.
        engine.set_now(1_050);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "WHO".into(),
            },
            &mut out,
            &mut fx,
        );
        let entries = out
            .events()
            .iter()
            .find_map(|(_, e)| match e {
                OutputEvent::Who(entries) => Some(entries.clone()),
                _ => None,
            })
            .expect("WHO output");
        let nick_row = entries.iter().find(|e| e.name == "Nick").expect("Nick row");
        assert_eq!(nick_row.idle_s, 40);
        assert_eq!(nick_row.conn_s, 50);
        // God ran WHO just now: idle 0, connected since setup at t=1000.
        let god_row = entries.iter().find(|e| e.name == "God").expect("God row");
        assert_eq!(god_row.idle_s, 0);
        assert_eq!(god_row.conn_s, 50);
    }

    #[test]
    fn slowtest_defers_effect_and_round_trips_apply() {
        let (mut engine, nick) = setup();
        // Wizard-only: Nick is denied and no effect is queued.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@slowtest".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(texts(&out).iter().any(|t| t == "Permission denied."));
        assert!(fx.is_empty());

        // God: "Thinking..." plus one deferred SlowTest effect.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(texts(&out).iter().any(|t| t == "Thinking..."));
        assert_eq!(fx.classes(), &[DeliveryClass::OnCommit]);
        let effects = fx.drain();
        let Some(Effect::SlowTest {
            actor,
            correlation,
            seconds,
        }) = effects.first().cloned()
        else {
            panic!("one SlowTest effect queued");
        };
        assert_eq!((actor, seconds), (GOD, 3));

        // The loop would sleep, then re-enter with the typed completion.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor,
                correlation,
                text: "Deep thought complete.".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(
            out.events().iter().any(|(aud, e)| matches!(aud, Audience::Actor(a) if *a == GOD)
                && matches!(e, OutputEvent::Sys(t) if t.to_plain_text() == "Deep thought complete."))
        );

        // Explicit seconds and validation.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest 10".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(matches!(
            fx.queued(),
            [Effect::SlowTest { seconds: 10, .. }]
        ));
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@slowtest bogus".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(fx.is_empty());
        assert!(texts(&out).iter().any(|t| t.contains("Usage: @slowtest")));
    }

    #[test]
    fn slowtest_done_for_missing_actor_is_dropped() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::SlowTestDone {
                actor: ObjId(99),
                correlation: 7,
                text: "ghost".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn module_async_done_routes_to_registered_completion_handler() {
        use crate::module::{AsyncCompletion, GameModule, ModError, Registrar};
        use std::sync::Arc;

        // A tiny module whose completion handler echoes the payload to the
        // actor and queues one persistence op — proving the world task's
        // persist side-channel is live during re-entry.
        struct EchoDone;
        impl AsyncCompletion for EchoDone {
            fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str) {
                if ctx.w.get(ctx.actor).is_none() {
                    return; // re-validate (treaty ruling 3)
                }
                ctx.out
                    .info(ctx.actor, format!("done #{correlation}: {payload}"));
                ctx.persist(ModuleOp {
                    ns: SmolStr::new("echo"),
                    op: crate::module::ModuleWrite::JsonSet {
                        key: SmolStr::new("last"),
                        value: serde_json::json!({ "payload": payload }),
                    },
                });
            }
        }
        struct EchoModule;
        impl GameModule for EchoModule {
            fn name(&self) -> &'static str {
                "echo"
            }
            fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
                reg.async_completion(Arc::new(EchoDone))
            }
        }

        let mut world = World::new_minimal();
        world.set_now(1_000);
        let mut registry = Registry::new();
        register_core_commands(&mut registry).unwrap();
        let mut engine = Engine::with_modules(world, registry, vec![Box::new(EchoModule)]).unwrap();

        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: SmolStr::new("echo"),
                actor: GOD,
                correlation: 42,
                payload: "hi".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(
            out.events()
                .iter()
                .any(|(aud, e)| matches!(aud, Audience::Actor(a) if *a == GOD)
                    && matches!(e, OutputEvent::Sys(t) if t.to_plain_text() == "done #42: hi"))
        );
        // The persist op reached the engine's module-op queue.
        let ops = engine.take_module_ops();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].ns, "echo");

        // A completion for an unregistered module is dropped, not a panic.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Apply(WorldCommand::ModuleAsyncDone {
                module: SmolStr::new("nobody"),
                actor: GOD,
                correlation: 1,
                payload: "x".into(),
            }),
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn wait_schedules_and_fires_through_engine() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        // now = 1000s. Queue a say for 3s out.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 3=say Delayed hello".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty(), "@wait queues silently");
        assert!(fx.is_empty(), "timers are not effects");
        assert_eq!(engine.sched.next_due(), Some(1_003_000));

        // Not due yet: nothing fires.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_002_999, &mut out, &mut fx);
        assert!(out.is_empty());
        assert_eq!(engine.sched.len(), 1);

        // Due: the line dispatches as Nick.
        engine.set_now(1_003);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_003_000, &mut out, &mut fx);
        assert!(out.events().iter().any(
            |(aud, e)| matches!(aud, Audience::Room { room, .. } if *room == LIMBO)
                && matches!(
                    e,
                    OutputEvent::Narrative { kind: NarrativeKind::Say, actor: Some(a), text }
                        if a.name == "Nick" && text.to_plain_text() == "Delayed hello"
                )
        ));
        assert!(engine.sched.is_empty());
    }

    #[test]
    fn fire_due_releases_only_the_bounded_slice_of_wait_slots() {
        use crate::sched::MAX_SCHEDULED_FIRES_PER_STEP;

        let (mut engine, _) = setup();
        let missing_actor = ObjId(u32::MAX);
        let total = MAX_SCHEDULED_FIRES_PER_STEP + 1;
        for i in 0..total {
            engine.sched.insert(
                1_001_000,
                ScheduledKind::WaitCmd {
                    actor: missing_actor,
                    line: format!("say timer-{i:03}"),
                },
            );
        }
        engine.pending_waits.insert(missing_actor, total as u32);

        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_001_000, &mut out, &mut fx);
        assert!(out.is_empty(), "waits for a missing actor are silent");
        assert_eq!(engine.sched.len(), 1);
        assert_eq!(engine.sched.next_due(), Some(1_001_000));
        assert_eq!(engine.pending_waits.get(&missing_actor), Some(&1));

        engine.fire_due(1_001_000, &mut out, &mut fx);
        assert!(engine.sched.is_empty());
        assert!(!engine.pending_waits.contains_key(&missing_actor));
    }

    #[test]
    fn wait_validates_arguments() {
        let (mut engine, _) = setup();
        for bad in [
            "@wait",
            "@wait 3",
            "@wait 3=",
            "@wait x=say hi",
            "@wait 99999999999=say hi",
        ] {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(
                Inbound::Cmd {
                    conn: 1,
                    actor: GOD,
                    line: bad.into(),
                },
                &mut out,
                &mut fx,
            );
            assert!(
                engine.sched.is_empty(),
                "{bad:?} must not schedule anything"
            );
            assert!(
                texts(&out).iter().any(|t| t.contains("wait")),
                "{bad:?} must error"
            );
        }
    }

    #[test]
    fn wait_fired_for_destroyed_actor_is_dropped() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 1=say too late".into(),
            },
            &mut out,
            &mut fx,
        );
        engine.world.destroy(nick).unwrap();
        engine.world.sweep();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_001_000, &mut out, &mut fx);
        assert!(out.is_empty());
    }

    #[test]
    fn wait_queue_is_capped_per_actor() {
        let (mut engine, nick) = setup();
        for _ in 0..WAIT_QUEUE_MAX {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(
                Inbound::Cmd {
                    conn: 2,
                    actor: nick,
                    line: "@wait 60=say tick".into(),
                },
                &mut out,
                &mut fx,
            );
            assert!(out.events().is_empty(), "under the cap queues silently");
        }
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX);

        // One past the cap: refused with an error, nothing scheduled.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 60=say overflow".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(
            texts(&out)
                .iter()
                .any(|t| t.contains("Too many commands queued.")),
            "over-cap @wait must error"
        );
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX);

        // Another actor is unaffected by Nick's full queue.
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "@wait 60=say fine".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty());
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX + 1);

        // Firing releases slots: after the queue drains, @wait works again.
        engine.set_now(1_100);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.fire_due(1_100_000, &mut out, &mut fx);
        assert!(engine.sched.is_empty());
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "@wait 60=say again".into(),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.events().is_empty(), "slots must free after firing");
        assert_eq!(engine.sched.len(), 1);
    }

    #[test]
    fn wait_queue_refusal_survives_a_dirty_step_as_best_effort() {
        let (mut engine, nick) = setup();
        engine
            .registry
            .register(
                CommandSpec {
                    name: "mixedwait".into(),
                    shape: ArgShape::None,
                    help: "test-only dirty step plus wait".into(),
                    ..CommandSpec::default()
                },
                Arc::new(|ctx: &mut Ctx<'_>| {
                    ctx.w.set_attr(
                        ctx.actor,
                        "mixedwait",
                        crate::attr::Attr::text("committed", ctx.actor),
                    )?;
                    ctx.out.defer(Deferred::ScheduleIn {
                        delay_ms: 60_000,
                        kind: ScheduledKind::WaitCmd {
                            actor: ctx.actor,
                            line: "look".to_string(),
                        },
                    });
                    Ok(())
                }),
            )
            .unwrap();
        for _ in 0..WAIT_QUEUE_MAX {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(
                Inbound::Cmd {
                    conn: 2,
                    actor: nick,
                    line: "@wait 60=look".into(),
                },
                &mut out,
                &mut fx,
            );
        }
        let _ = engine.world.take_dirty();

        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Cmd {
                conn: 2,
                actor: nick,
                line: "mixedwait".into(),
            },
            &mut out,
            &mut fx,
        );

        assert!(engine.world.take_dirty().contains(&nick));
        assert_eq!(out.classes(), &[DeliveryClass::BestEffort]);
        assert!(out.classes()[0].releases_on(DeliveryOutcome::Committed));
        assert!(out.classes()[0].releases_on(DeliveryOutcome::Aborted));
        assert!(
            texts(&out)
                .iter()
                .any(|line| line == "Too many commands queued.")
        );
        assert_eq!(engine.sched.len() as u32, WAIT_QUEUE_MAX);
    }

    #[test]
    fn control_shutdown_announces_and_stops() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        let outcome = engine.step(Inbound::Control(ControlMsg::Shutdown), &mut out, &mut fx);
        assert_eq!(outcome, StepOutcome::Shutdown);
        assert!(
            out.events()
                .iter()
                .any(|(aud, e)| matches!(aud, Audience::All)
                    && matches!(e, OutputEvent::Sys(t) if t.to_plain_text().contains("Shutdown")))
        );
    }

    #[test]
    fn control_dump_announces_and_continues() {
        let (mut engine, _) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        let outcome = engine.step(Inbound::Control(ControlMsg::Dump), &mut out, &mut fx);
        assert_eq!(outcome, StepOutcome::Continue);
        assert!(
            out.events()
                .iter()
                .any(|(aud, e)| matches!(aud, Audience::All)
                    && matches!(e, OutputEvent::Sys(t) if t.to_plain_text().contains("Dumping")))
        );
    }

    #[test]
    fn connect_announcement_audience_excludes_only_the_actor() {
        // Regression guard on announcement audience shape. (DARK-aware
        // suppression is a later session-layer concern; a DARK player's
        // connect still queues the room event today.)
        let (mut engine, nick) = setup();
        engine.world.set_flag(nick, Flags::DARK, true).unwrap();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: false,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        let (aud, _) = &out.events()[0];
        assert_eq!(
            aud,
            &Audience::Room {
                room: LIMBO,
                exclude: vec![nick],
                subject: Some(nick),
            }
        );
    }

    // ------------------------------------------------------------------
    // Inbound::Query → Reply (the web Req hydration seam, M4)
    // ------------------------------------------------------------------

    /// A test query handler: echoes its params back inside a `Sys` event,
    /// or refuses when params carry `"deny": true`.
    struct EchoQuery;

    impl crate::module::QueryHandler for EchoQuery {
        fn query(
            &self,
            _w: &World,
            actor: ObjId,
            params: &serde_json::Value,
        ) -> Result<OutputEvent, RichText> {
            if params.get("deny").and_then(serde_json::Value::as_bool) == Some(true) {
                return Err(RichText::tag("error", "Denied by handler."));
            }
            Ok(OutputEvent::Sys(RichText::plain(format!(
                "echo actor={actor} params={params}"
            ))))
        }
    }

    fn engine_with_echo_query() -> (Engine, ObjId) {
        let (mut engine, nick) = setup();
        crate::module::Registrar::new(&mut engine.registry, "test")
            .query_handler("test.echo", Arc::new(EchoQuery))
            .unwrap();
        (engine, nick)
    }

    fn query(name: &str, args: serde_json::Value) -> Inbound {
        Inbound::Query {
            conn: 42,
            actor: GOD,
            id: 7,
            name: SmolStr::new(name),
            args,
        }
    }

    /// The full round trip: a Query dispatches to the registered handler
    /// and exactly one conn-addressed `Reply { id }` wrapping the handler's
    /// event comes back.
    #[test]
    fn query_round_trips_to_conn_scoped_reply() {
        let (mut engine, _) = engine_with_echo_query();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            query("test.echo", serde_json::json!({"n": 3})),
            &mut out,
            &mut fx,
        );
        assert_eq!(out.len(), 1, "exactly one reply: {:?}", out.events());
        let (aud, ev) = &out.events()[0];
        assert_eq!(aud, &Audience::Conn(42), "reply is conn-scoped");
        let OutputEvent::Reply { id, event } = ev else {
            panic!("expected Reply, got {ev:?}");
        };
        assert_eq!(*id, 7);
        let OutputEvent::Sys(t) = event.as_ref() else {
            panic!("expected wrapped Sys, got {event:?}");
        };
        assert_eq!(t.to_plain_text(), r#"echo actor=#1 params={"n":3}"#);
        assert!(fx.is_empty());
    }

    /// A handler refusal and an unknown query name still answer with a
    /// correlated Reply (wrapping a Sys error), never silence.
    #[test]
    fn query_errors_reply_with_correlated_sys() {
        let (mut engine, _) = engine_with_echo_query();
        for (q, needle) in [
            (
                query("test.echo", serde_json::json!({"deny": true})),
                "Denied by handler.",
            ),
            (query("no.such", serde_json::json!({})), "Unknown query"),
        ] {
            let (mut out, mut fx) = (Outbox::new(), Effects::new());
            engine.step(q, &mut out, &mut fx);
            assert_eq!(out.len(), 1);
            let (aud, ev) = &out.events()[0];
            assert_eq!(aud, &Audience::Conn(42));
            let OutputEvent::Reply { id: 7, event } = ev else {
                panic!("expected Reply id 7, got {ev:?}");
            };
            assert!(
                matches!(event.as_ref(), OutputEvent::Sys(t) if t.to_plain_text().contains(needle)),
                "wanted {needle:?} in {event:?}"
            );
        }
    }

    /// The core `"who"` handler is registered by `register_core_commands`
    /// and answers with the same listing the WHO command produces. Queries
    /// never stamp `last_input` (panel refreshes must not reset idle).
    #[test]
    fn who_query_answers_without_stamping_idle() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::ConnUp {
                conn: 2,
                actor: nick,
                caps_structured: true,
                from_linkdead: false,
            },
            &mut out,
            &mut fx,
        );
        engine.set_now(1_040);
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Query {
                conn: 2,
                actor: nick,
                id: 1,
                name: SmolStr::new("who"),
                args: serde_json::json!({}),
            },
            &mut out,
            &mut fx,
        );
        let (aud, ev) = &out.events()[0];
        assert_eq!(aud, &Audience::Conn(2));
        let OutputEvent::Reply { id: 1, event } = ev else {
            panic!("expected Reply, got {ev:?}");
        };
        let OutputEvent::Who(entries) = event.as_ref() else {
            panic!("expected Who, got {event:?}");
        };
        let row = entries.iter().find(|e| e.name == "Nick").expect("Nick row");
        assert_eq!(row.conn_s, 40);
        assert_eq!(
            row.idle_s, 40,
            "a Query must not stamp last_input (idle would read 0)"
        );
    }

    /// A Query for a vanished actor is dropped outright — no reply, no
    /// panic (the connection is mid-teardown).
    #[test]
    fn query_for_missing_actor_is_dropped() {
        let (mut engine, _) = engine_with_echo_query();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Query {
                conn: 3,
                actor: ObjId(99),
                id: 9,
                name: SmolStr::new("test.echo"),
                args: serde_json::json!({}),
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
    }
}
