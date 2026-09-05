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
use crate::authoritative::AuthoritativeBundle;
use crate::cmd::{
    ArgShape, CmdError, CommandSpec, Ctx, ParsedArgs, Registry, RegistryError, SwitchSet,
    dispatch_with_action_state,
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
    // ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
    /// One authoritative action a command handler selected through
    /// [`Ctx::submit_authoritative`](crate::cmd::Ctx::submit_authoritative).
    ///
    /// This is the whole `Ctx`-level action submit 08j §5 (iii) and 08i §2 row
    /// 5 named as missing. It is a **selector, not an offer**: the handler
    /// names a registered action definition and its parameters, and the driving
    /// loop regenerates that actor's own projected offer, resolves it, and
    /// enters `ActionCoordinator::prepare_and_stage_resolved` — the identical
    /// five steps the `WorldMsg::AuthoritativeAction` arm takes. A handler can
    /// therefore neither construct an opaque `ActionOfferId` nor bypass
    /// projection, and **a second submission in one command is a refusal**
    /// ([`Engine::drain_deferred`] stages nothing at all in that case).
    AuthoritativeSubmit(Box<AuthoritativeSubmit>),
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

// ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----

/// One command-originated authoritative action, awaiting the driving loop.
///
/// The handler that produced it performs **no mutation of its own**: the whole
/// mechanic is the registered [`ActionProvider`](crate::ActionProvider)'s, and
/// the actor's confirmation is released only by the durable receipt (§4's
/// standing checklist item 1). `key` is the handler's own deterministic
/// discriminator for the command; it enters the request identity so a retyped
/// identical command inside one process epoch is a preflight HIT rather than a
/// second commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeSubmit {
    /// The acting object. The driving loop projects **this actor's** offers.
    pub actor: ObjId,
    /// The registered action definition to select.
    pub action: crate::mechanics::DefinitionRef,
    /// The subject the handler expects the projected offer to carry. A
    /// disagreement is a refusal, never a silent re-target.
    pub subject: crate::action::PerceivedRef,
    /// Bounded target list, exactly as the offer's target schema admits.
    pub targets: Vec<crate::action::PerceivedRef>,
    /// Submitted parameters, validated against the offer's own schema.
    pub parameters: serde_json::Value,
    /// The command's deterministic idempotency discriminator.
    pub key: crate::action::IdempotencyKeyText,
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
    /// Someone started or stopped composing a line (interface plan I4,
    /// D-I-11): a presence hint the engine fans out to the actor's room as
    /// [`OutputEvent::Composing`], minus the actor, gated by perception
    /// like a room line, never for a DARK actor, never for one who opted
    /// out (`pref:composing` set to `off`), and never an idle stamp. The
    /// bridge rate-limits it; an `active: false` always passes.
    Composing {
        /// The connection it arrived on.
        conn: ConnId,
        /// The actor typing.
        actor: ObjId,
        /// `true` when a draft began, `false` when it was sent or abandoned.
        active: bool,
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
    persistence: StagedActionPersistence,
    installation: PreparedInstallation,
    guards: GuardSet,
}

/// Persistence identity paired with one non-cloneable prepared RAM image.
pub enum StagedActionPersistence {
    /// K2 compatibility lane using the process-local writer serial.
    Legacy(Vec<ModuleOp>),
    /// K3 lane carrying the complete immutable exact-token intent.
    Authoritative(Box<AuthoritativeBundle>),
}

impl fmt::Debug for StagedActionPersistence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy(module_ops) => f.debug_tuple("Legacy").field(module_ops).finish(),
            Self::Authoritative(bundle) => f.debug_tuple("Authoritative").field(bundle).finish(),
        }
    }
}

impl fmt::Debug for StagedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagedAction")
            .field("persistence", &self.persistence)
            .field("installation", &self.installation)
            .field("guards", &self.guards)
            .finish()
    }
}

impl StagedAction {
    /// Split the already-built persistence writes from the post-ack images.
    pub fn into_parts(self) -> (StagedActionPersistence, PreparedInstallation) {
        (self.persistence, self.installation)
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
    #[error("authoritative bundle provider does not own the prepared transaction")]
    AuthoritativeProviderMismatch,
    #[error("authoritative bundle idempotency scope does not belong to its provider")]
    AuthoritativeScopeMismatch,
    #[error("authoritative bundle writes do not exactly match the prepared transaction")]
    AuthoritativeWriteSetMismatch,
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
    /// Process-local durable key ring for opaque legal-action offers. The
    /// deployment loader installs it before module boot; it is never part of
    /// world persistence or a projected DTO.
    offer_keys: Option<crate::offer::OfferKeyRing>,
    /// Process-local durable key ring for authoritative resolution RNG. Only
    /// short-lived [`PrepareCtx`](crate::resolution::PrepareCtx) capabilities
    /// cross into providers; the ring and root bytes never do.
    resolution_keys: Option<crate::resolution::ResolutionKeyRing>,
    /// Process-local HMAC key ring for durable delayed-decision prompts.
    decision_keys: Option<crate::decision::DecisionKeyRing>,
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
    /// E7c2 B1: at most one command-originated authoritative submission
    /// awaiting [`Engine::take_authoritative_submit`]. One is the whole queue —
    /// the store admits one unresolved authoritative intent per backend
    /// (D-E7-39), so a second is back-pressure, not a buffer.
    authoritative_submit: Option<Box<AuthoritativeSubmit>>,
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
            offer_keys: None,
            resolution_keys: None,
            decision_keys: None,
            staged_action: None,
            module_ops: Vec::new(),
            scene_ticks: HashMap::new(),
            durable_gate: None,
            conns: HashMap::new(),
            conn_count: HashMap::new(),
            pending_waits: HashMap::new(),
            lag_last_ms: 0,
            lag_max_ms: 0,
            authoritative_submit: None,
        }
    }

    /// Install or atomically replace the process-local offer key ring. A
    /// replacement is used for bounded online rotation; existing tokens remain
    /// valid while their key version is retained by the new ring.
    pub fn set_offer_keys(&mut self, keys: crate::offer::OfferKeyRing) {
        self.offer_keys = Some(keys);
    }

    /// Install or atomically replace the process-local resolution key ring.
    /// Accepted actions carry their draw transcript, so replay never depends
    /// on rerunning the provider with this mutable current version.
    pub fn set_resolution_keys(&mut self, keys: crate::resolution::ResolutionKeyRing) {
        self.resolution_keys = Some(keys);
    }

    /// Install or atomically replace the delayed-decision token key ring.
    pub fn set_decision_keys(&mut self, keys: crate::decision::DecisionKeyRing) {
        self.decision_keys = Some(keys);
    }

    /// Root-secret versions still referenced by open or durably closed
    /// action windows. Feed this exact set into reference-aware key rotation.
    #[must_use]
    pub fn resolution_key_rotation_references(&self) -> std::collections::BTreeSet<u32> {
        self.action_state
            .unresolved_window_resolution_secret_versions()
    }

    /// Read one exact typed clock through the kernel's built-in clocks or the
    /// composition-frozen module provider registry.
    ///
    /// Mutable clock generations fail closed: callers cannot ask for an old
    /// world-IC generation and accidentally receive a value under the current
    /// rate, and registered providers must explicitly accept the requested
    /// generation before the kernel constructs the returned stamp.
    pub fn clock_now(
        &self,
        clock: &crate::clock::ClockRef,
    ) -> Result<crate::clock::ClockStamp, crate::clock_provider::ClockReadError> {
        match clock {
            crate::clock::ClockRef::Utc => Ok(crate::clock::ClockStamp {
                clock: crate::clock::ClockRef::Utc,
                at: self.world.now(),
            }),
            crate::clock::ClockRef::WorldIc { generation } => {
                if *generation == 0 {
                    return Err(crate::clock_provider::ClockReadError::ZeroGeneration {
                        clock: clock.clone(),
                    });
                }
                let current = self.world.world_ic_now();
                let crate::clock::ClockRef::WorldIc {
                    generation: current_generation,
                } = current.clock
                else {
                    unreachable!("world timeline must return a world-IC stamp")
                };
                if *generation != current_generation {
                    return Err(crate::clock_provider::ClockReadError::StaleGeneration {
                        requested: *generation,
                        current: current_generation,
                    });
                }
                Ok(current)
            }
            crate::clock::ClockRef::SceneTurn { .. } => self
                .registry
                .clock_providers()
                .ok_or(crate::clock_provider::ClockReadError::RegistryUnavailable)?
                .read_scene(&self.world, &self.action_state, clock),
            crate::clock::ClockRef::DomainTick { .. } => self
                .registry
                .clock_providers()
                .ok_or(crate::clock_provider::ClockReadError::RegistryUnavailable)?
                .read_domain(&self.world, &self.action_state, clock),
            crate::clock::ClockRef::Registered { .. } => self
                .registry
                .clock_providers()
                .ok_or(crate::clock_provider::ClockReadError::RegistryUnavailable)?
                .read(&self.world, &self.action_state, clock),
        }
    }

    /// Read one ruleset-owned resource through its composition-frozen,
    /// read-only adapter.
    pub fn resource_snapshot(
        &self,
        resource: &crate::resource::ResourceRef,
    ) -> Result<
        Option<crate::resource_provider::ResourceSnapshot>,
        crate::resource_provider::ResourceReadError,
    > {
        self.registry
            .resource_effect_providers()
            .ok_or(crate::resource_provider::ResourceReadError::RegistryUnavailable)?
            .read_resource(&self.world, &self.action_state, resource)
    }

    /// Read one exact effect instance through its definition-owned adapter.
    pub fn effect_instance(
        &self,
        definition: &crate::mechanics::DefinitionRef,
        target: &crate::resource::EffectTarget,
        instance: crate::resource::EffectInstanceId,
    ) -> Result<Option<crate::resource::EffectInstance>, crate::resource_provider::EffectReadError>
    {
        self.registry
            .resource_effect_providers()
            .ok_or(crate::resource_provider::EffectReadError::RegistryUnavailable)?
            .read_effect(
                &self.world,
                &self.action_state,
                definition,
                target,
                instance,
            )
    }

    /// Discover a bounded canonical page of durable schedules that need an
    /// authoritative action under the requested drive cause.
    pub fn discover_scheduled_actions(
        &self,
        cause: crate::durable_schedule::ScheduleDriveCause,
        cursor: Option<&crate::schedule_source::ScheduleSourceCursor>,
        limit: usize,
    ) -> Result<
        crate::schedule_source::ScheduleSourcePage,
        crate::schedule_source::ScheduleSourceScanError,
    > {
        self.registry
            .schedule_sources()
            .ok_or(crate::schedule_source::ScheduleSourceScanError::RegistryUnavailable)?
            .scan(self, cause, cursor, limit)
    }

    /// Discover one bounded canonical page of ready or expired-lease delayed
    /// decisions. Prompt linkage is authenticated before a candidate leaves
    /// the engine.
    pub fn discover_decision_dispatches(
        &self,
        cursor: Option<&crate::decision_source::DecisionSourceCursor>,
        limit: usize,
    ) -> Result<
        crate::decision_source::DecisionSourcePage,
        crate::decision_source::DecisionSourceScanError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionSourceScanError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionSourceScanError::RegistryUnavailable)?
            .scan(self, keys, cursor, limit)
    }

    /// Resolve one durable outbox wake directly against its authoritative
    /// pending/prompt/deadline records. A stale terminal wake is a no-op.
    pub fn decision_dispatch_for_wake(
        &self,
        owner: &crate::mechanics::OwnerId,
        wake: &crate::decision::DecisionWake,
    ) -> Result<
        Option<crate::decision_source::DecisionDispatchCandidate>,
        crate::decision_source::DecisionSourceScanError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionSourceScanError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionSourceScanError::RegistryUnavailable)?
            .candidate_for_wake(self, keys, owner, wake)
    }

    /// Construct stable internal ingress for one discovered lease attempt.
    pub fn decision_lease_ingress(
        &self,
        candidate: &crate::decision_source::DecisionDispatchCandidate,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionLeaseIngressError>
    {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionSourceCandidateError::RegistryMismatch)?
            .ingress_for_candidate(candidate, idempotency_epoch)
    }

    /// Authenticate one provider completion against its exact live lease and
    /// construct the canonical ordinary-action ingress it will contend on.
    pub fn decision_response_ingress(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        response: &crate::decision::DecisionResponse,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionResponseIngressError>
    {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?
            .response_ingress(self, keys, fence, response)
    }

    /// Run the source's composition-branded deterministic fallback selector
    /// against only the exact persisted projected prompt.
    pub fn select_decision_fallback(
        &self,
        candidate: &crate::decision_source::DecisionFallbackCandidate,
    ) -> Result<
        crate::decision::DecisionResponse,
        crate::decision_source::DecisionFallbackSelectionError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionFallbackSelectionError::StaleCandidate)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionFallbackSelectionError::StaleCandidate)?
            .select_fallback(self, keys, candidate)
    }

    /// Construct the ordinary semantic action ingress selected by one exact
    /// durable fallback candidate.
    pub fn decision_fallback_ingress(
        &self,
        candidate: &crate::decision_source::DecisionFallbackCandidate,
        response: &crate::decision::DecisionResponse,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionResponseIngressError>
    {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?
            .fallback_ingress(self, keys, candidate, response)
    }

    /// Regenerate and resolve the exact prompt-time offer selected by an
    /// authenticated completion, restoring its private revision witnesses.
    pub fn resolve_decision_response(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        response: &crate::decision::DecisionResponse,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionResponseResolution,
        crate::decision_source::DecisionResponseResolveError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?
            .resolve_response(self, keys, fence, response, ingress)
    }

    pub fn resolve_decision_fallback(
        &self,
        candidate: &crate::decision_source::DecisionFallbackCandidate,
        response: &crate::decision::DecisionResponse,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionResponseResolution,
        crate::decision_source::DecisionResponseResolveError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResponseIngressError::RegistryUnavailable)?
            .resolve_fallback(self, keys, candidate, response, ingress)
    }

    /// Construct stable owner-local ingress for binding an already durable
    /// selected ordinary action to its exact pending decision.
    pub fn decision_resolving_ingress(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        binding: &crate::decision::DecisionActionBinding,
        outcome: crate::decision::DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionResolvingIngressError>
    {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResolvingIngressError::RegistryUnavailable)?
            .resolving_ingress(self, fence, binding, outcome, idempotency_epoch)
    }

    /// Construct stable owner-local ingress for committing one typed provider
    /// attempt result without including diagnostic text in its identity.
    pub fn decision_attempt_ingress(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        kind: crate::decision::DecisionAttemptFailureKind,
        max_attempts: u8,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionAttemptIngressError>
    {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionAttemptIngressError::RegistryUnavailable)?
            .attempt_failure_ingress(self, fence, kind, max_attempts, idempotency_epoch)
    }

    pub fn plan_decision_attempt_failure(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        kind: crate::decision::DecisionAttemptFailureKind,
        max_attempts: u8,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionAttemptPlan,
        crate::decision_source::DecisionAttemptPlanError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionAttemptPlanError::RegistryUnavailable)?
            .plan_attempt_failure(self, fence, kind, max_attempts, ingress)
    }

    pub fn decision_fallback_trigger_for_dispatch(
        &self,
        candidate: &crate::decision_source::DecisionDispatchCandidate,
    ) -> Result<
        crate::decision_source::DecisionFallbackTriggerCandidate,
        crate::decision_source::DecisionFallbackTriggerError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionFallbackTriggerError::RegistryUnavailable)?
            .fallback_trigger_for_dispatch(self, candidate)
    }

    pub fn decision_fallback_trigger_ingress(
        &self,
        candidate: &crate::decision_source::DecisionFallbackTriggerCandidate,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionFallbackTriggerError>
    {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionFallbackTriggerError::RegistryUnavailable)?
            .fallback_trigger_ingress(self, candidate, idempotency_epoch)
    }

    pub fn plan_decision_fallback_trigger(
        &self,
        candidate: &crate::decision_source::DecisionFallbackTriggerCandidate,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionFallbackTriggerPlan,
        crate::decision_source::DecisionFallbackTriggerPlanError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionFallbackTriggerPlanError::RegistryUnavailable)?
            .plan_fallback_trigger(self, candidate, ingress)
    }

    pub fn decision_cancellation_ingress(
        &self,
        controller: &crate::decision::DecisionControllerId,
        decision: crate::decision::DecisionId,
        revision: u64,
        idempotency_epoch: u64,
    ) -> Result<
        crate::action::IngressEnvelope,
        crate::decision_source::DecisionCancellationIngressError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionCancellationIngressError::RegistryUnavailable)?
            .cancellation_ingress(self, controller, decision, revision, idempotency_epoch)
    }

    pub fn plan_decision_cancellation(
        &self,
        controller: &crate::decision::DecisionControllerId,
        decision: crate::decision::DecisionId,
        revision: u64,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionCancellationPlan,
        crate::decision_source::DecisionCancellationPlanError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionCancellationPlanError::RegistryUnavailable)?
            .plan_cancellation(self, controller, decision, revision, ingress)
    }

    /// Re-read and plan an exact owner-local Leased-to-Resolving transition
    /// after its lifecycle ingress has passed store preflight.
    pub fn plan_decision_resolving(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        binding: &crate::decision::DecisionActionBinding,
        outcome: crate::decision::DecisionTerminalOutcome,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionResolvingPlan,
        crate::decision_source::DecisionResolvingPlanError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionResolvingPlanError::RegistryUnavailable)?
            .plan_resolving(self, fence, binding, outcome, ingress)
    }

    /// Construct stable owner-local ingress for terminalizing one exact
    /// resolving decision binding.
    pub fn decision_terminal_ingress(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        binding: &crate::decision::DecisionActionBinding,
        outcome: crate::decision::DecisionTerminalOutcome,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionTerminalIngressError>
    {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionTerminalIngressError::RegistryUnavailable)?
            .terminal_ingress(self, fence, binding, outcome, idempotency_epoch)
    }

    /// Re-read and plan one exact owner-local Resolving-to-Terminal
    /// transition after its lifecycle ingress passes store preflight.
    pub fn plan_decision_terminal(
        &self,
        fence: &crate::decision::DecisionCompletionFence,
        binding: &crate::decision::DecisionActionBinding,
        outcome: crate::decision::DecisionTerminalOutcome,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionTerminalPlan,
        crate::decision_source::DecisionTerminalPlanError,
    > {
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionTerminalPlanError::RegistryUnavailable)?
            .plan_terminal(self, fence, binding, outcome, ingress)
    }

    /// Construct stable owner-local ingress for replacing one exact terminal
    /// decision with its compact attribution receipt.
    pub fn decision_compaction_ingress(
        &self,
        candidate: &crate::decision_source::DecisionCompactionCandidate,
        idempotency_epoch: u64,
    ) -> Result<crate::action::IngressEnvelope, crate::decision_source::DecisionCompactionError>
    {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionCompactionError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionCompactionError::RegistryUnavailable)?
            .compaction_ingress(self, keys, candidate, idempotency_epoch)
    }

    pub fn plan_decision_compaction(
        &self,
        candidate: &crate::decision_source::DecisionCompactionCandidate,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionCompactionPlan,
        crate::decision_source::DecisionCompactionError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionCompactionError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionCompactionError::RegistryUnavailable)?
            .plan_compaction(self, keys, candidate, ingress)
    }

    /// Re-read and plan one exact decision lease after store preflight.
    pub fn plan_decision_dispatch(
        &self,
        candidate: &crate::decision_source::DecisionDispatchCandidate,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::decision_source::DecisionDispatchPlan,
        crate::decision_source::DecisionSourcePlanError,
    > {
        let keys = self
            .decision_keys
            .as_ref()
            .ok_or(crate::decision_source::DecisionSourcePlanError::RegistryUnavailable)?;
        self.registry
            .decision_sources()
            .ok_or(crate::decision_source::DecisionSourcePlanError::RegistryUnavailable)?
            .plan_candidate(self, keys, candidate, ingress)
    }

    /// Ask the source's trusted read-only adapter to build domain addressing,
    /// parameters, and guards after exact store preflight.
    pub fn plan_scheduled_action(
        &self,
        candidate: &crate::schedule_source::ScheduledActionCandidate,
        ingress: &crate::action::IngressEnvelope,
    ) -> Result<
        crate::schedule_source::ScheduledActionPlan,
        crate::schedule_source::ScheduleActionPlanError,
    > {
        self.registry
            .schedule_sources()
            .ok_or(crate::schedule_source::ScheduleActionPlanError::RegistryUnavailable)?
            .plan_candidate(self, candidate, ingress)
    }

    /// Enumerate the current viewer-safe legal actions through the frozen
    /// provider registry and deployment offer key ring.
    pub fn project_legal_actions(
        &self,
        query: &crate::offer::LegalActionQuery,
    ) -> Result<crate::offer::ProjectedLegalActionSink, crate::offer::LegalActionProjectionError>
    {
        let keys = self
            .offer_keys
            .as_ref()
            .ok_or(crate::offer::LegalActionProjectionError::KeyRingUnavailable)?;
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::offer::LegalActionProjectionError::RegistryUnavailable)?;
        providers.project_legal_actions(self, query, keys)
    }

    /// Capture the private prompt-time witnesses for one exact projected
    /// delayed-decision offer. Providers name coordinates, while the kernel
    /// supplies current generations and proves every cross-owner bridge.
    fn capture_decision_offer_witness(
        &self,
        action: &crate::mechanics::DefinitionRef,
        query: &crate::offer::LegalActionQuery,
        offer: &crate::offer::ProjectedLegalAction,
    ) -> Result<
        (crate::decision::PendingOfferWitness, i32),
        crate::decision::DecisionWitnessCaptureError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::decision::DecisionWitnessCaptureError::ProviderRegistryUnavailable)?;
        let definitions = self
            .registry
            .definitions()
            .ok_or(crate::decision::DecisionWitnessCaptureError::MechanicsRegistryUnavailable)?;
        let bridges = self
            .registry
            .bridges()
            .ok_or(crate::decision::DecisionWitnessCaptureError::MechanicsRegistryUnavailable)?;
        let revisions = self
            .registry
            .revision_validators()
            .ok_or(crate::decision::DecisionWitnessCaptureError::RevisionRegistryUnavailable)?;
        let (keys, bridge_definitions, rank) = providers.delayed_decision_witness_keys(
            action,
            query,
            offer,
            self,
            definitions,
            bridges,
        )?;
        let context = crate::action::RevisionValidatorContext::new(&self.world, &self.action_state);
        let stamp = |key: crate::action::RevisionKey| -> Result<
            crate::action::RevisionStamp,
            crate::decision::DecisionWitnessCaptureError,
        > {
            let generation = revisions
                .current_generation(&context, &key)?
                .ok_or_else(|| {
                    crate::decision::DecisionWitnessCaptureError::MissingRevision(Box::new(
                        key.clone(),
                    ))
                })?;
            Ok(crate::action::RevisionStamp { key, generation })
        };
        let common_revisions = keys
            .common
            .into_iter()
            .map(&stamp)
            .collect::<Result<Vec<_>, _>>()?;
        let target_revisions = keys
            .targets
            .into_iter()
            .map(|target| {
                let revisions = target
                    .revisions
                    .into_iter()
                    .map(&stamp)
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(crate::decision::PendingTargetWitness {
                    perceived: target.perceived,
                    revisions,
                })
            })
            .collect::<Result<Vec<_>, crate::decision::DecisionWitnessCaptureError>>()?;
        Ok((
            crate::decision::PendingOfferWitness {
                offer: offer.offer(),
                common_revisions,
                target_revisions,
                bridge_definitions,
            },
            rank,
        ))
    }

    /// Project, witness, rank, and bound one delayed-decision action set in a
    /// single immutable-world pass. Ranking sees projected data only; exact
    /// definition and offer identities provide deterministic tie-breaks.
    pub fn project_decision_offers(
        &self,
        query: &crate::offer::LegalActionQuery,
    ) -> Result<
        crate::decision::DecisionOfferProjection,
        crate::decision::DecisionWitnessCaptureError,
    > {
        let projected = self.project_legal_actions(query)?;
        let mut ranked = Vec::with_capacity(projected.len());
        for internal in projected.internal() {
            let (witness, rank) = self.capture_decision_offer_witness(
                internal.action(),
                query,
                internal.projected(),
            )?;
            ranked.push((
                rank,
                internal.action().clone(),
                internal.projected().clone(),
                witness,
            ));
        }
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.offer().cmp(&right.2.offer()))
        });
        ranked.truncate(crate::decision::MAX_DECISION_OFFERS);
        let (actions, offered): (Vec<_>, Vec<_>) = ranked
            .into_iter()
            .map(|(_, _, action, witness)| (action, witness))
            .unzip();
        Ok(crate::decision::DecisionOfferProjection::new(
            actions, offered,
        ))
    }

    /// Resolve an untrusted perceived reference into the private truth query
    /// consumed by action providers.
    ///
    /// Known objects must still be ordinarily visible and carry the one
    /// unambiguous current station-grant epoch. Opaque contacts instead route
    /// through their exact registered witness provider; only that trusted
    /// provider may reveal the truth subject and mint the authority guard that
    /// is retained in every resulting offer.
    pub fn legal_action_query(
        &self,
        principal: crate::action::PrincipalId,
        actor: ObjId,
        subject: crate::action::PerceivedRef,
    ) -> Result<crate::offer::LegalActionQuery, crate::offer::LegalActionSubjectError> {
        const BOOTSTRAP_SELF_PROJECTION_GENERATION: u64 = 1;
        const BOOTSTRAP_STAFF_PROJECTION_GENERATION: u64 = 1;

        let (truth_subject, projection_generation, witness) = match &subject {
            crate::action::PerceivedRef::SelfActor => (
                crate::action::SubjectKey::Object { id: actor },
                BOOTSTRAP_SELF_PROJECTION_GENERATION,
                None,
            ),
            crate::action::PerceivedRef::KnownObject { id, revision } => {
                if self.world.get(*id).is_none()
                    || !module::can_perceive_with_action_state(
                        &self.registry,
                        &self.world,
                        actor,
                        *id,
                        &self.action_state,
                    )
                {
                    return Err(crate::offer::LegalActionSubjectError::Invalid);
                }
                // Administrative providers may expose a visible object as a
                // neutral staff subject without fabricating a control-seat
                // grant. The provider still owns the role/legality check. Any
                // action requiring control channels continues through the
                // exact station-grant selection below and will not project.
                if crate::perm::has_role(&self.world, actor, crate::Role::Royalty)
                    && *revision == BOOTSTRAP_STAFF_PROJECTION_GENERATION
                {
                    (
                        crate::action::SubjectKey::Object { id: *id },
                        BOOTSTRAP_STAFF_PROJECTION_GENERATION,
                        None,
                    )
                } else {
                    let mut epochs = self
                        .world
                        .command_subjects_of(actor)
                        .iter()
                        .filter(|grant| grant.subject == *id)
                        .map(|grant| grant.epoch);
                    let Some(epoch) = epochs.next() else {
                        return Err(crate::offer::LegalActionSubjectError::Invalid);
                    };
                    if epoch != *revision || epochs.any(|candidate| candidate != epoch) {
                        return Err(crate::offer::LegalActionSubjectError::Invalid);
                    }
                    (crate::action::SubjectKey::Object { id: *id }, epoch, None)
                }
            }
            crate::action::PerceivedRef::Contact {
                provider,
                id,
                generation,
            } => {
                let validators = self
                    .registry
                    .authority_guard_validators()
                    .ok_or(crate::offer::LegalActionSubjectError::Unavailable)?;
                let context =
                    crate::action::AuthorityGuardContext::new(&self.world, &self.action_state);
                let request = crate::action::AuthorityWitnessRequest::Contact {
                    provider: provider.clone(),
                    principal: principal.clone(),
                    id: *id,
                    generation: *generation,
                };
                let issued = match validators.witness(&context, &request) {
                    Ok(Some(witness)) => witness,
                    Ok(None) | Err(crate::action::AuthorityWitnessError::Missing { .. }) => {
                        return Err(crate::offer::LegalActionSubjectError::Invalid);
                    }
                    Err(_) => return Err(crate::offer::LegalActionSubjectError::Unavailable),
                };
                let crate::action::AuthorityWitness::Contact {
                    truth_subject,
                    guard,
                } = issued
                else {
                    return Err(crate::offer::LegalActionSubjectError::Unavailable);
                };
                (truth_subject, *generation, Some(guard))
            }
        };

        let query = crate::offer::LegalActionQuery::new(
            principal,
            actor,
            truth_subject,
            subject,
            projection_generation,
            None,
        )
        .map_err(|_| crate::offer::LegalActionSubjectError::Unavailable)?;
        Ok(match witness {
            Some(guard) => query.with_authority_guard(guard),
            None => query,
        })
    }

    /// Resolve one untrusted opaque submission against a freshly regenerated
    /// internal set without exposing the deployment key ring to providers or
    /// interface code.
    pub fn resolve_legal_action(
        &self,
        offers: &crate::offer::ProjectedLegalActionSink,
        mut execution: crate::offer::OfferExecutionContext,
        submission: crate::offer::OfferSubmission,
    ) -> Result<crate::offer::ResolvedAuthoritativeOffer, crate::offer::LegalActionResolutionError>
    {
        let keys = self
            .offer_keys
            .as_ref()
            .ok_or(crate::offer::LegalActionResolutionError::KeyRingUnavailable)?;
        if let Some(expires) = offers
            .expiration_for(keys, submission.offer)
            .map_err(crate::offer::LegalActionResolutionError::from)?
        {
            execution.now = Some(self.clock_now(&expires.clock).map_err(|_| {
                crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                )
            })?);
        }
        let resolved = offers
            .resolve(keys, execution, submission)
            .map_err(crate::offer::LegalActionResolutionError::from)?;
        let (action, invocation, truth_subject, guards) = resolved.into_resolution_parts();
        let principal = invocation.principal().clone();
        let actor = invocation.actor();
        let mut truth_targets = Vec::with_capacity(invocation.targets().len());
        let mut target_guards = Vec::new();
        for target in invocation.targets() {
            let (truth, guard) = match target {
                crate::action::PerceivedRef::SelfActor => {
                    (crate::action::SubjectKey::Object { id: actor }, None)
                }
                crate::action::PerceivedRef::KnownObject { id, .. } => {
                    // The regenerated offer HMAC binds the provider-controlled
                    // public revision. Recheck current existence/perception as
                    // well; unlike an operated subject, a target needs no
                    // station grant merely to be targetable.
                    if self.world.get(*id).is_none()
                        || !module::can_perceive_with_action_state(
                            &self.registry,
                            &self.world,
                            actor,
                            *id,
                            &self.action_state,
                        )
                    {
                        return Err(crate::offer::LegalActionResolutionError::Resolve(
                            crate::offer::OfferResolveError::Unavailable,
                        ));
                    }
                    (crate::action::SubjectKey::Object { id: *id }, None)
                }
                crate::action::PerceivedRef::Contact {
                    provider,
                    id,
                    generation,
                } => {
                    let validators = self.registry.authority_guard_validators().ok_or(
                        crate::offer::LegalActionResolutionError::Resolve(
                            crate::offer::OfferResolveError::Unavailable,
                        ),
                    )?;
                    let context =
                        crate::action::AuthorityGuardContext::new(&self.world, &self.action_state);
                    let request = crate::action::AuthorityWitnessRequest::Contact {
                        provider: provider.clone(),
                        principal: principal.clone(),
                        id: *id,
                        generation: *generation,
                    };
                    let issued = validators.witness(&context, &request).map_err(|_| {
                        crate::offer::LegalActionResolutionError::Resolve(
                            crate::offer::OfferResolveError::Unavailable,
                        )
                    })?;
                    let Some(crate::action::AuthorityWitness::Contact {
                        truth_subject,
                        guard,
                    }) = issued
                    else {
                        return Err(crate::offer::LegalActionResolutionError::Resolve(
                            crate::offer::OfferResolveError::Unavailable,
                        ));
                    };
                    (truth_subject, Some(guard))
                }
            };
            truth_targets.push(truth);
            if let Some(guard) = guard
                && !guards.authority().contains(&guard)
                && !target_guards.contains(&guard)
            {
                target_guards.push(guard);
            }
        }
        let guards = guards.with_authority_guards(&target_guards).map_err(|_| {
            crate::offer::LegalActionResolutionError::Resolve(
                crate::offer::OfferResolveError::Unavailable,
            )
        })?;
        let resolved =
            crate::action::ResolvedActionInvocation::new(invocation, truth_subject, truth_targets)
                .ok_or(crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                ))?;
        let definitions = self.registry.definitions().ok_or(
            crate::offer::LegalActionResolutionError::Resolve(
                crate::offer::OfferResolveError::Unavailable,
            ),
        )?;
        let bridges =
            self.registry
                .bridges()
                .ok_or(crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                ))?;
        let (revision_keys, bridge_definitions) = self
            .registry
            .action_providers()
            .ok_or(crate::offer::LegalActionResolutionError::Resolve(
                crate::offer::OfferResolveError::Unavailable,
            ))?
            .resolved_revision_keys(&action, &resolved, definitions, bridges)
            .map_err(|_| {
                crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                )
            })?;
        let mut revision_stamps = Vec::with_capacity(revision_keys.len());
        if !revision_keys.is_empty() {
            let validators = self.registry.revision_validators().ok_or(
                crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                ),
            )?;
            let context =
                crate::action::RevisionValidatorContext::new(&self.world, &self.action_state);
            for key in revision_keys {
                let Some(generation) =
                    validators.current_generation(&context, &key).map_err(|_| {
                        crate::offer::LegalActionResolutionError::Resolve(
                            crate::offer::OfferResolveError::Unavailable,
                        )
                    })?
                else {
                    return Err(crate::offer::LegalActionResolutionError::Resolve(
                        crate::offer::OfferResolveError::Unavailable,
                    ));
                };
                revision_stamps.push(crate::action::RevisionStamp { key, generation });
            }
        }
        let guards = guards
            .with_definition_guards(bridge_definitions, definitions)
            .and_then(|guards| guards.with_revision_stamps(revision_stamps))
            .map_err(|_| {
                crate::offer::LegalActionResolutionError::Resolve(
                    crate::offer::OfferResolveError::Unavailable,
                )
            })?;
        Ok(crate::offer::ResolvedAuthoritativeOffer::new(
            action, resolved, guards,
        ))
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
            if name == crate::clock::WORLD_TIMELINE_NAMESPACE {
                return Err(ModError::msg(format!(
                    "module name {name:?} is reserved for engine-owned world timeline state"
                )));
            }
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
            let typed_state = crate::action_state::ModuleActionState::for_module(
                Some(&self.action_state),
                Some(module_name.as_str()),
            );
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module_name),
                typed_state,
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

    /// Prepare one registered provider's typed post-images and semantic K3
    /// result material. Persistence identity remains coordinator-owned.
    pub fn prepare_registered_authoritative_action(
        &mut self,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
        )
    }

    /// Prepare an action after the engine has privately resolved its subject
    /// and selected targets. This is the production path for contact-targeted
    /// actions; the projected invocation remains the durable request identity.
    pub fn prepare_registered_resolved_authoritative_action(
        &mut self,
        action: &crate::mechanics::DefinitionRef,
        resolved: &crate::action::ResolvedActionInvocation,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_resolved_authoritative_invocation(
            action,
            resolved,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
        )
    }

    /// Prepare a scheduled action while the kernel seeds its exact durable
    /// cursor post-image into the same owner-scoped transaction before the
    /// provider runs. Re-evaluation against current state makes a stale
    /// discovery non-mutating and prevents providers from forgetting to
    /// advance (or independently advancing) the authoritative cursor.
    pub fn prepare_registered_scheduled_authoritative_action(
        &mut self,
        candidate: &crate::schedule_source::ScheduledActionCandidate,
        invocation: &crate::action::ActionInvocation,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let schedules = self
            .registry
            .schedule_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation_seeded(
            candidate.action(),
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                schedules.seed_advance(candidate, txn).map_err(|source| {
                    crate::action::ActionProviderPrepareError::Schedule(Box::new(source))
                })
            },
        )
    }

    /// Prepare the registered owner action while the kernel seeds one exact
    /// delayed-decision lease into the same transaction. The returned worker
    /// lease remains caller custody and must not be spawned before the action's
    /// exact authoritative receipt is acknowledged and installed.
    pub fn prepare_registered_decision_lease_action(
        &mut self,
        candidate: &crate::decision_source::DecisionDispatchCandidate,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionLeaseTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (
            PreparedTxn,
            crate::action::PreparedAuthoritativeAction,
            crate::decision::DecisionWorkerLease,
        ),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let mut worker = None;
        let (prepared, output) = providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                worker = Some(decisions.seed_lease(candidate, transition, txn).map_err(
                    |source| {
                        crate::action::ActionProviderPrepareError::DecisionLease(Box::new(source))
                    },
                )?);
                Ok(())
            },
        )?;
        Ok((
            prepared,
            output,
            worker.expect("successful decision lease seed returns worker custody"),
        ))
    }

    /// Prepare the registered controller-owner action while the kernel seeds
    /// the exact Leased-to-Resolving post-image into the same transaction.
    pub fn prepare_registered_decision_resolving_action(
        &mut self,
        fence: &crate::decision::DecisionCompletionFence,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionResolvingTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                decisions
                    .seed_resolving(fence, transition, txn)
                    .map_err(|source| {
                        crate::action::ActionProviderPrepareError::DecisionLifecycle(Box::new(
                            source,
                        ))
                    })
            },
        )
    }

    /// Prepare the registered controller-owner action while the kernel seeds
    /// an exact Resolving-to-Terminal post-image in the same transaction.
    pub fn prepare_registered_decision_terminal_action(
        &mut self,
        fence: &crate::decision::DecisionCompletionFence,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionTerminalTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                decisions
                    .seed_terminal(fence, transition, txn)
                    .map_err(|source| {
                        crate::action::ActionProviderPrepareError::DecisionTerminal(Box::new(
                            source,
                        ))
                    })
            },
        )
    }

    /// Prepare the registered owner action while the kernel atomically
    /// inserts a compact receipt and removes the terminal decision bundle.
    pub fn prepare_registered_decision_compaction_action(
        &mut self,
        candidate: &crate::decision_source::DecisionCompactionCandidate,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionCompactionTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                decisions
                    .seed_compaction(candidate, transition, txn)
                    .map_err(|source| {
                        crate::action::ActionProviderPrepareError::DecisionCompaction(Box::new(
                            source,
                        ))
                    })
            },
        )
    }

    /// Prepare the registered controller-owner action while the kernel seeds
    /// an exact provider-attempt result into the same transaction.
    pub fn prepare_registered_decision_attempt_action(
        &mut self,
        fence: &crate::decision::DecisionCompletionFence,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionAttemptTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (
            PreparedTxn,
            crate::action::PreparedAuthoritativeAction,
            Option<crate::decision::DecisionCompletionFence>,
        ),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let mut fallback_fence = None;
        let (prepared, output) = providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                fallback_fence = decisions
                    .seed_attempt_failure(fence, transition, txn)
                    .map_err(|source| {
                        crate::action::ActionProviderPrepareError::DecisionAttempt(Box::new(source))
                    })?;
                Ok(())
            },
        )?;
        Ok((prepared, output, fallback_fence))
    }

    /// Prepare the owner action while atomically transferring exact decision
    /// custody to deterministic fallback.
    pub fn prepare_registered_decision_fallback_trigger_action(
        &mut self,
        candidate: &crate::decision_source::DecisionFallbackTriggerCandidate,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionFallbackTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (
            PreparedTxn,
            crate::action::PreparedAuthoritativeAction,
            crate::decision::DecisionCompletionFence,
        ),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let mut fallback_fence = None;
        let (prepared, output) = providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                fallback_fence = Some(
                    decisions
                        .seed_fallback_trigger(candidate, transition, txn)
                        .map_err(|source| {
                            crate::action::ActionProviderPrepareError::DecisionFallbackTrigger(
                                Box::new(source),
                            )
                        })?,
                );
                Ok(())
            },
        )?;
        Ok((
            prepared,
            output,
            fallback_fence.expect("seed callback records fallback fence"),
        ))
    }

    /// Prepare the owner action while atomically installing the exact
    /// Terminal(cancelled) post-image.
    pub fn prepare_registered_decision_cancellation_action(
        &mut self,
        controller: &crate::decision::DecisionControllerId,
        action: &crate::mechanics::DefinitionRef,
        invocation: &crate::action::ActionInvocation,
        transition: crate::decision::DecisionCancellationTransition,
        commit_token: crate::action::CommitToken,
    ) -> Result<
        (PreparedTxn, crate::action::PreparedAuthoritativeAction),
        crate::action::ActionProviderPrepareError,
    > {
        let providers = self
            .registry
            .action_providers()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        let decisions = self
            .registry
            .decision_sources()
            .ok_or(crate::action::ActionProviderPrepareError::RegistryUnavailable)?;
        providers.prepare_authoritative_invocation_seeded(
            action,
            invocation,
            commit_token,
            &mut self.action_state,
            self.resolution_keys.as_ref(),
            |txn| {
                decisions
                    .seed_cancellation(controller, transition, txn)
                    .map_err(|source| {
                        crate::action::ActionProviderPrepareError::DecisionCancellation(Box::new(
                            source,
                        ))
                    })
            },
        )
    }

    /// Release a prepared transaction that could not be assembled into a
    /// complete authoritative bundle. This is distinct from aborting a
    /// [`PreparedInstallation`]: coordinators need this form when canonical
    /// hashing or bundle validation fails before staging consumes the
    /// transaction.
    pub fn abort_prepared_transaction(&mut self, prepared: PreparedTxn) {
        let (_, installation) = prepared.into_parts();
        self.action_state.abort(installation);
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
            persistence: StagedActionPersistence::Legacy(module_ops),
            installation,
            guards,
        });
        Ok(())
    }

    /// Stage one K3 action only when its immutable replay writes exactly match
    /// the typed post-images retained for post-ack RAM installation.
    ///
    /// This deliberately accepts an already-complete bundle: outcome,
    /// causality, witnesses, definitions, and outbox intent must all be fixed
    /// before the synchronous final guard check transfers store custody.
    pub fn stage_authoritative_action(
        &mut self,
        prepared: PreparedTxn,
        guards: GuardSet,
        bundle: AuthoritativeBundle,
    ) -> Result<(), StagePreparedActionError> {
        let refusal = if self.staged_action.is_some() {
            Some(StagePreparedActionError::AlreadyStaged)
        } else if prepared.owner() != bundle.provider() {
            Some(StagePreparedActionError::AuthoritativeProviderMismatch)
        } else if bundle.idempotency().scope().owner != *bundle.provider() {
            Some(StagePreparedActionError::AuthoritativeScopeMismatch)
        } else if prepared.authoritative_replay_writes() != bundle.writes() {
            Some(StagePreparedActionError::AuthoritativeWriteSetMismatch)
        } else if self.world.has_dirty() {
            Some(StagePreparedActionError::DirtyWorld)
        } else if !self.module_ops.is_empty() {
            Some(StagePreparedActionError::LegacyModuleOps)
        } else {
            None
        };
        if let Some(error) = refusal {
            let (_, installation) = prepared.into_parts();
            self.action_state.abort(installation);
            return Err(error);
        }
        let (_, installation) = prepared.into_parts();
        self.staged_action = Some(StagedAction {
            persistence: StagedActionPersistence::Authoritative(Box::new(bundle)),
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
        let revalidation = self.revalidate_guards(&staged.guards);
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

    /// Pure pre-preparation guard check used when a retained asynchronous
    /// action may already be stale on arrival. The driver still repeats this
    /// check at staged-action transfer to close the final synchronous race.
    pub fn revalidate_guards(
        &self,
        guards: &GuardSet,
    ) -> Result<GuardRevalidation, StagedActionAdmissionError> {
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
        let definitions =
            self.registry
                .definitions()
                .ok_or(StagedActionAdmissionError::RegistryUnavailable {
                    component: "definition",
                })?;
        guards
            .revalidate_with_action_state(
                &self.world,
                &self.action_state,
                revisions,
                authority,
                definitions,
            )
            .map_err(StagedActionAdmissionError::Revalidation)
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
        module::emit_world_event_with_action_state(
            &self.registry,
            &mut self.world,
            out,
            ev,
            Some(&self.action_state),
        );
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
                    dispatch_with_action_state(
                        &self.registry,
                        &mut self.world,
                        actor,
                        &line,
                        out,
                        Some(&self.action_state),
                    );
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
            Inbound::Composing {
                conn,
                actor,
                active,
            } => {
                self.handle_composing(conn, actor, active, out);
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
                        dispatch_with_action_state(
                            &self.registry,
                            &mut self.world,
                            actor,
                            &line,
                            out,
                            Some(&self.action_state),
                        );
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
            let typed_state = crate::action_state::ModuleActionState::for_module(
                Some(&self.action_state),
                Some(module.as_str()),
            );
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module.clone()),
                typed_state,
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
            let typed_state = crate::action_state::ModuleActionState::for_module(
                Some(&self.action_state),
                Some(module.as_str()),
            );
            let mut ctx = Ctx {
                w: &mut self.world,
                actor: GOD,
                switches: SwitchSet::default(),
                args: ParsedArgs::default(),
                out,
                reg: &self.registry,
                module: Some(module.clone()),
                typed_state,
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
                    module::fire_conn_hook_with_action_state(
                        &self.registry,
                        &mut self.world,
                        out,
                        actor,
                        true,
                        Some(&self.action_state),
                    );
                    module::emit_world_event_with_action_state(
                        &self.registry,
                        &mut self.world,
                        out,
                        &WorldEvent::Connected { who: actor },
                        Some(&self.action_state),
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
            crate::commands::show_room_with_action_state(
                &self.registry,
                &self.world,
                actor,
                room,
                out,
                Some(&self.action_state),
            );
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
                module::fire_conn_hook_with_action_state(
                    &self.registry,
                    &mut self.world,
                    out,
                    actor,
                    false,
                    Some(&self.action_state),
                );
                module::emit_world_event_with_action_state(
                    &self.registry,
                    &mut self.world,
                    out,
                    &WorldEvent::Disconnected { who: actor },
                    Some(&self.action_state),
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
            module::fire_conn_hook_with_action_state(
                &self.registry,
                &mut self.world,
                out,
                actor,
                false,
                Some(&self.action_state),
            );
            module::emit_world_event_with_action_state(
                &self.registry,
                &mut self.world,
                out,
                &WorldEvent::Disconnected { who: actor },
                Some(&self.action_state),
            );
        }
        self.world.session_end(actor);
    }

    /// Answer one [`Inbound::Query`]: look up the registered
    /// [`QueryHandler`](crate::module::QueryHandler) by `name`, run it
    /// read-only as `actor`, and address the wrapped
    /// [`OutputEvent::Reply`] to the requesting connection only. Every
    /// Fan a composing hint out to the actor's room (interface plan I4,
    /// D-I-11). Presence must not become surveillance or an idle exploit:
    /// the hint never touches `last_input`, a DARK actor emits nothing, an
    /// actor whose `pref:composing` attribute reads `off` emits nothing,
    /// the actor is excluded from the audience, and the fan-out carries
    /// them as its perception subject so a concealed typist stays
    /// concealed. Best-effort delivery: a lost frame costs nothing, the
    /// clients expire their pills on a timer.
    fn handle_composing(&self, conn: ConnId, actor: ObjId, active: bool, out: &mut Outbox) {
        let Some(o) = self.world.get(actor) else {
            tracing::debug!(%conn, %actor, "Composing for missing actor dropped");
            return;
        };
        if o.flags.contains(crate::flags::Flags::DARK) {
            return;
        }
        if crate::commands::attr_text(&self.world, actor, COMPOSING_PREF_ATTR)
            .is_some_and(|v| composing_opted_out(&v))
        {
            return;
        }
        let Some(room) = o.location else {
            return;
        };
        out.push_classified(
            DeliveryClass::BestEffort,
            crate::Audience::Room {
                room,
                exclude: vec![actor],
                subject: Some(actor),
            },
            OutputEvent::Composing {
                actor: crate::obj_ref(&self.world, actor),
                active,
            },
        );
    }

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
            Some((module, handler)) => {
                let context = crate::module::HookReadContext::for_module(
                    &self.world,
                    Some(&self.action_state),
                    module,
                )
                .with_registry(&self.registry);
                match handler.query_with_context(&context, actor, args) {
                    Ok(event) => event,
                    Err(err) => OutputEvent::Sys(err),
                }
            }
            // A registered panel feed with no same-named query handler is a
            // push-only feed (`wod.roll`): a subscribe to it has nothing to
            // hydrate and simply acknowledges, so both bridges can forward
            // any feed name unchanged (interface plan I1, D-I-3).
            None if self
                .registry
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed.as_str() == name) =>
            {
                OutputEvent::Sys(RichText::tag(
                    "system",
                    format!("Subscribed to panel feed {name:?}."),
                ))
            }
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
                module::apply_async_completion_with_action_state(
                    &self.registry,
                    &mut self.world,
                    out,
                    module,
                    actor,
                    correlation,
                    &payload,
                    Some(&self.action_state),
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
        // E7c2 B1: how many authoritative submissions *this* drain saw. A
        // handler that submits twice fans out, so the second one voids the
        // first: nothing at all is staged and the actor is told plainly.
        let mut submits = 0_usize;
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
                Deferred::AuthoritativeSubmit(request) => {
                    submits = submits.saturating_add(1);
                    if submits > 1 {
                        // Fan-out: void whatever this drain queued, so the
                        // command stages NOTHING rather than an arbitrary half.
                        self.authoritative_submit = None;
                        out.best_effort_error(
                            request.actor,
                            "That command tried to submit more than one authoritative action; nothing was staged.",
                        );
                        continue;
                    }
                    if self.authoritative_submit.is_some() {
                        // One unresolved authoritative intent per backend
                        // (D-E7-39). This is back-pressure, not a buffer.
                        out.best_effort_error(
                            request.actor,
                            "An earlier action is still committing; try again in a moment.",
                        );
                        continue;
                    }
                    self.authoritative_submit = Some(request);
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

    /// E7c2 B1: whether one command-originated authoritative submission is
    /// waiting for the driving loop. The loop arms its drain on this.
    #[must_use]
    pub const fn has_authoritative_submit(&self) -> bool {
        self.authoritative_submit.is_some()
    }

    /// E7c2 B1: take the one waiting command-originated authoritative
    /// submission. The driving loop owns every later step; nothing here
    /// projects, resolves, prepares or persists.
    pub fn take_authoritative_submit(&mut self) -> Option<Box<AuthoritativeSubmit>> {
        self.authoritative_submit.take()
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
/// The player attribute that opts out of composing hints (interface plan
/// I4, §8 q1: on by default, off per player).
pub const COMPOSING_PREF_ATTR: &str = "pref:composing";

/// Whether a `pref:composing` value opts out: `off`, `no`, `false`, `0`,
/// `none` (case-insensitive); anything else keeps the default.
#[must_use]
pub fn composing_opted_out(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "off" | "no" | "false" | "0" | "none"
    )
}

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
    fn staff_known_object_subject_needs_no_fabricated_station_grant() {
        use crate::action::{PerceivedRef, PrincipalId, SubjectKey};
        use crate::offer::LegalActionSubjectError;

        let (engine, nick) = setup();
        let perceived = PerceivedRef::KnownObject {
            id: nick,
            revision: 1,
        };
        let query = engine
            .legal_action_query(PrincipalId::Object { id: GOD }, GOD, perceived.clone())
            .unwrap();
        assert_eq!(query.truth_subject(), &SubjectKey::Object { id: nick });
        assert_eq!(query.projected_subject(), &perceived);
        assert!(query.authority_guards().is_empty());

        assert_eq!(
            engine
                .legal_action_query(
                    PrincipalId::Object { id: nick },
                    nick,
                    PerceivedRef::KnownObject {
                        id: GOD,
                        revision: 1,
                    },
                )
                .unwrap_err(),
            LegalActionSubjectError::Invalid
        );
        assert_eq!(
            engine
                .legal_action_query(
                    PrincipalId::Object { id: GOD },
                    GOD,
                    PerceivedRef::KnownObject {
                        id: nick,
                        revision: 2,
                    },
                )
                .unwrap_err(),
            LegalActionSubjectError::Invalid
        );
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

    #[test]
    fn contact_subject_resolution_is_private_current_and_guarded() {
        use std::sync::Mutex;

        use crate::action::{AuthorityGuard, ContactId, PerceivedRef, PrincipalId, SubjectKey};
        use crate::action_state::{StateFamilyHandle, StateFamilyId};
        use crate::clock::{ClockRef, ClockStamp};
        use crate::contact::{
            ContactGuardValidator, ContactState, ContactStateCodec, ContactStateDraft,
            contact_state_family_descriptor,
        };
        use crate::mechanics::{CanonicalId, OwnerId};

        struct ContactModule {
            captured: Arc<Mutex<Option<StateFamilyHandle<ContactId, ContactState>>>>,
        }

        impl GameModule for ContactModule {
            fn name(&self) -> &'static str {
                "test_contacts"
            }

            fn register(&mut self, reg: &mut module::Registrar<'_>) -> Result<(), ModError> {
                let codec = ContactStateCodec;
                let handle = reg.state_family(
                    contact_state_family_descriptor(StateFamilyId::new("contacts").unwrap())
                        .unwrap(),
                    codec,
                    codec,
                    codec,
                    codec,
                )?;
                reg.authority_guard_validator(
                    crate::action::AuthorityGuardKind::Contact,
                    ContactGuardValidator::new(handle.clone()),
                )?;
                *self.captured.lock().unwrap() = Some(handle);
                Ok(())
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(ContactModule {
                captured: Arc::clone(&captured),
            })],
        )
        .unwrap();
        let handle = captured.lock().unwrap().clone().unwrap();
        let provider = OwnerId::new("test_contacts").unwrap();
        let principal = PrincipalId::Object { id: GOD };
        let id = ContactId::from_bytes([0x43; 16]);
        let observed_at = ClockStamp {
            clock: ClockRef::DomainTick {
                owner: provider.clone(),
                id: CanonicalId::new("field").unwrap(),
                generation: 4,
            },
            at: 10,
        };
        let contact = ContactState::new(ContactStateDraft {
            truth_subject: SubjectKey::Object { id: ObjId(90) },
            audience: vec![principal.clone()],
            observed_at: observed_at.clone(),
        })
        .unwrap();
        let mut transaction = engine.action_transaction(&handle).unwrap();
        transaction.insert(&handle, &id, contact.clone()).unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        engine.install_prepared_action(installation);

        let perceived = PerceivedRef::Contact {
            provider: provider.clone(),
            id,
            generation: 1,
        };
        let query = engine
            .legal_action_query(principal.clone(), GOD, perceived.clone())
            .unwrap();
        assert_eq!(query.truth_subject(), contact.truth_subject());
        assert_eq!(query.projected_subject(), &perceived);
        assert_eq!(
            query.authority_guards(),
            &[AuthorityGuard::Contact {
                provider: provider.clone(),
                principal: principal.clone(),
                id,
                generation: 1,
            }]
        );
        assert_eq!(
            engine
                .legal_action_query(PrincipalId::Object { id: ObjId(2) }, GOD, perceived.clone(),)
                .unwrap_err(),
            crate::offer::LegalActionSubjectError::Invalid
        );

        let lost = contact
            .checked_mark_lost(
                1,
                ClockStamp {
                    at: 11,
                    ..observed_at
                },
                CanonicalId::new("occluded").unwrap(),
            )
            .unwrap();
        let mut transaction = engine.action_transaction(&handle).unwrap();
        transaction.replace(&handle, &id, lost).unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        engine.install_prepared_action(installation);
        assert_eq!(
            engine
                .legal_action_query(principal, GOD, perceived)
                .unwrap_err(),
            crate::offer::LegalActionSubjectError::Invalid
        );
    }

    #[test]
    fn module_command_reads_only_its_own_typed_family() {
        use std::sync::Mutex;

        use crate::action_state::{
            ActionStateError, ModuleActionState, ModuleActionStateReadError, StateFamilyDescriptor,
            StateFamilyHandle, StateFamilyId,
        };
        use crate::mechanics::{CanonicalId, IncidentId, OwnerId, RecoveryReason};

        struct TypedReaderModule {
            captured: Arc<Mutex<Option<StateFamilyHandle<u32, u32>>>>,
        }

        struct TypedQuery {
            handle: StateFamilyHandle<u32, u32>,
        }

        impl crate::module::QueryHandler for TypedQuery {
            fn query(
                &self,
                _world: &World,
                _actor: ObjId,
                _params: &serde_json::Value,
            ) -> Result<OutputEvent, mush_proto::RichText> {
                Err(mush_proto::RichText::tag(
                    "error",
                    "typed query context missing",
                ))
            }

            fn query_with_context(
                &self,
                context: &crate::module::HookReadContext<'_>,
                _actor: ObjId,
                _params: &serde_json::Value,
            ) -> Result<OutputEvent, mush_proto::RichText> {
                let value = context
                    .action_state(&self.handle, &7)
                    .map_err(|error| mush_proto::RichText::tag("error", error.to_string()))?
                    .copied()
                    .ok_or_else(|| mush_proto::RichText::tag("error", "typed record missing"))?;
                Ok(OutputEvent::Sys(mush_proto::RichText::plain(format!(
                    "query-typed={value}"
                ))))
            }
        }

        impl GameModule for TypedReaderModule {
            fn name(&self) -> &'static str {
                "typed_reader"
            }

            fn register(&mut self, reg: &mut module::Registrar<'_>) -> Result<(), ModError> {
                reg.classify_commands(crate::CommandCompatibility::ReadOnly);
                let handle = reg.state_family(
                    StateFamilyDescriptor::new(
                        StateFamilyId::new("records").unwrap(),
                        CanonicalId::new("u32.decimal.v1").unwrap(),
                        CanonicalId::new("u32.value.v1").unwrap(),
                        1,
                        CanonicalId::new("u32.decode.v1").unwrap(),
                        CanonicalId::new("u32.validate.v1").unwrap(),
                        CanonicalId::new("u32.serialize.v1").unwrap(),
                    )
                    .unwrap(),
                    |key: &u32| Ok(key.to_string()),
                    |value: &serde_json::Value| {
                        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                    },
                    |_value: &u32| Ok(()),
                    |value: &u32| serde_json::to_value(value).map_err(|error| error.to_string()),
                )?;
                let command_handle = handle.clone();
                reg.command(
                    CommandSpec {
                        name: "typedread".into(),
                        ..CommandSpec::default()
                    },
                    Arc::new(move |ctx: &mut Ctx<'_>| {
                        let value = ctx
                            .action_state(&command_handle, &7)
                            .map_err(|error| CmdError::msg(error.to_string()))?
                            .copied()
                            .ok_or_else(|| CmdError::msg("typed record is missing"))?;
                        let page = ctx
                            .scan_action_state(&command_handle, None, 8)
                            .map_err(|error| CmdError::msg(error.to_string()))?;
                        ctx.out.info(
                            ctx.actor,
                            format!("typed={value}; records={}", page.entries().len()),
                        );
                        Ok(())
                    }),
                )?;
                reg.query_handler(
                    "typed.query",
                    Arc::new(TypedQuery {
                        handle: handle.clone(),
                    }),
                )?;
                *self.captured.lock().unwrap() = Some(handle);
                Ok(())
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            Registry::new(),
            vec![Box::new(TypedReaderModule {
                captured: Arc::clone(&captured),
            })],
        )
        .unwrap();
        let handle = captured.lock().unwrap().clone().unwrap();
        let mut transaction = engine.action_transaction(&handle).unwrap();
        transaction.insert(&handle, &7, 41).unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        engine.install_prepared_action(installation);

        let mut out = Outbox::new();
        let mut effects = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "typedread".into(),
            },
            &mut out,
            &mut effects,
        );
        assert!(texts(&out).iter().any(|line| line == "typed=41; records=1"));

        let mut query_out = Outbox::new();
        engine.step(
            Inbound::Query {
                conn: 1,
                actor: GOD,
                id: 77,
                name: "typed.query".into(),
                args: serde_json::json!({}),
            },
            &mut query_out,
            &mut effects,
        );
        assert!(query_out.events().iter().any(|(_, event)| matches!(
            event,
            OutputEvent::Reply { id: 77, event }
                if matches!(event.as_ref(), OutputEvent::Sys(text) if text.to_plain_text() == "query-typed=41")
        )));

        let foreign =
            ModuleActionState::for_module(Some(engine.action_state()), Some("different_owner"));
        assert!(matches!(
            foreign.get(&handle, &7),
            Err(ModuleActionStateReadError::ForeignOwner { .. })
        ));
        assert!(matches!(
            ModuleActionState::unavailable().get(&handle, &7),
            Err(ModuleActionStateReadError::Unavailable)
        ));

        let own = ModuleActionState::for_module(Some(engine.action_state()), Some("typed_reader"));
        engine
            .registry
            .freeze_mechanics_owner(
                &OwnerId::new("typed_reader").unwrap(),
                IncidentId(701),
                RecoveryReason::IntegrityViolation {
                    code: CanonicalId::new("typed_read_freeze_test").unwrap(),
                },
            )
            .unwrap();
        assert!(matches!(
            own.get(&handle, &7),
            Err(ModuleActionStateReadError::State(
                ActionStateError::OwnerNotReady(_)
            ))
        ));
    }

    #[test]
    fn engine_move_hooks_receive_owner_scoped_live_typed_state() {
        use std::sync::Mutex;

        use crate::action_state::{
            ModuleActionStateReadError, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
        };
        use crate::mechanics::CanonicalId;
        use crate::module::{EventHook, HookPoint, HookReadContext};

        struct TypedHook {
            handle: StateFamilyHandle<u32, u32>,
        }

        impl EventHook for TypedHook {
            fn move_attempt(
                &self,
                ctx: &HookReadContext<'_>,
                _actor: ObjId,
                _exit: Option<ObjId>,
                _dest: ObjId,
            ) -> Result<(), mush_proto::RichText> {
                match ctx.action_state(&self.handle, &7) {
                    Ok(Some(41)) => Err(mush_proto::RichText::plain("typed veto")),
                    Err(ModuleActionStateReadError::Unavailable) => {
                        Err(mush_proto::RichText::plain("typed unavailable"))
                    }
                    other => Err(mush_proto::RichText::plain(format!(
                        "unexpected typed hook state: {other:?}"
                    ))),
                }
            }
        }

        struct TypedHookModule {
            captured: Arc<Mutex<Option<StateFamilyHandle<u32, u32>>>>,
        }

        impl GameModule for TypedHookModule {
            fn name(&self) -> &'static str {
                "typed_hook"
            }

            fn register(&mut self, reg: &mut module::Registrar<'_>) -> Result<(), ModError> {
                let handle = reg.state_family(
                    StateFamilyDescriptor::new(
                        StateFamilyId::new("records").unwrap(),
                        CanonicalId::new("u32.decimal.v1").unwrap(),
                        CanonicalId::new("u32.value.v1").unwrap(),
                        1,
                        CanonicalId::new("u32.decode.v1").unwrap(),
                        CanonicalId::new("u32.validate.v1").unwrap(),
                        CanonicalId::new("u32.serialize.v1").unwrap(),
                    )
                    .unwrap(),
                    |key: &u32| Ok(key.to_string()),
                    |value: &serde_json::Value| {
                        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                    },
                    |_value: &u32| Ok(()),
                    |value: &u32| serde_json::to_value(value).map_err(|error| error.to_string()),
                )?;
                reg.hook(
                    HookPoint::MoveAttempt,
                    Arc::new(TypedHook {
                        handle: handle.clone(),
                    }),
                );
                *self.captured.lock().unwrap() = Some(handle);
                Ok(())
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut world = World::new_minimal();
        let destination = world.create(ObjKind::Room, "North", GOD, None).unwrap();
        let exit = world
            .create(ObjKind::Exit, "north", GOD, Some(LIMBO))
            .unwrap();
        world.link_exit(exit, Some(destination)).unwrap();
        let mut engine = Engine::with_modules(
            world,
            Registry::new(),
            vec![Box::new(TypedHookModule {
                captured: Arc::clone(&captured),
            })],
        )
        .unwrap();
        let handle = captured.lock().unwrap().clone().unwrap();
        let mut transaction = engine.action_transaction(&handle).unwrap();
        transaction.insert(&handle, &7, 41).unwrap();
        let (_, installation) = transaction.prepare().unwrap().into_parts();
        engine.install_prepared_action(installation);

        let mut out = Outbox::new();
        let mut effects = Effects::new();
        engine.step(
            Inbound::Cmd {
                conn: 1,
                actor: GOD,
                line: "north".into(),
            },
            &mut out,
            &mut effects,
        );
        assert_eq!(
            engine.world.get(GOD).and_then(|obj| obj.location),
            Some(LIMBO)
        );
        assert!(texts(&out).iter().any(|line| line == "typed veto"));

        let compatibility = crate::module::check_move_attempt(
            &engine.registry,
            &engine.world,
            GOD,
            Some(exit),
            destination,
        )
        .unwrap_err();
        assert_eq!(compatibility.to_plain_text(), "typed unavailable");
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

    /// A registered panel feed with no same-named query handler is
    /// push-only: subscribing to it by name acknowledges instead of erring,
    /// so the bridges can forward any manifest feed unchanged (interface
    /// plan I1, D-I-3). An unregistered name is still "Unknown query".
    #[test]
    fn subscribe_to_push_only_feed_acks_by_name() {
        let (mut engine, _) = engine_with_echo_query();
        crate::module::Registrar::new(&mut engine.registry, "pushy")
            .panel_feed("pushy.stream")
            .expect("feed registers");
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            query("pushy.stream", serde_json::json!({})),
            &mut out,
            &mut fx,
        );
        assert_eq!(out.len(), 1);
        let (_, ev) = &out.events()[0];
        let OutputEvent::Reply { id: 7, event } = ev else {
            panic!("expected Reply id 7, got {ev:?}");
        };
        let OutputEvent::Sys(t) = event.as_ref() else {
            panic!("expected Sys ack, got {event:?}");
        };
        assert!(
            t.to_plain_text().contains("Subscribed to panel feed"),
            "push-only feed acks: {t:?}"
        );
        assert_eq!(
            t.spans()[0].tag.as_deref(),
            Some("system"),
            "an ack is not an error"
        );
    }

    /// `commands.list` reaches the registry through the read context the
    /// engine attaches (interface plan I1, D-I-5): the reply is a keyframe on
    /// the `commands.list` feed listing the core verbs the actor may use.
    /// (Queries never stamp `last_input`; the `who` test below pins that for
    /// the whole query path.)
    #[test]
    fn commands_list_query_answers_a_bounded_keyframe() {
        let (mut engine, nick) = setup();
        let (mut out, mut fx) = (Outbox::new(), Effects::new());
        engine.step(
            Inbound::Query {
                conn: 42,
                actor: nick,
                id: 9,
                name: SmolStr::new("commands.list"),
                args: serde_json::json!({}),
            },
            &mut out,
            &mut fx,
        );
        assert_eq!(out.len(), 1, "{:?}", out.events());
        let OutputEvent::Reply { id: 9, event } = &out.events()[0].1 else {
            panic!("expected Reply id 9, got {:?}", out.events());
        };
        let OutputEvent::Panel { feed, data, .. } = event.as_ref() else {
            panic!("expected a Panel keyframe, got {event:?}");
        };
        assert_eq!(feed.as_str(), "commands.list");
        let names: Vec<&str> = data["commands"]
            .as_array()
            .expect("commands array")
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        assert!(
            names.contains(&"WHO") && names.contains(&"help"),
            "{names:?}"
        );
        assert_eq!(data["truncated"], serde_json::json!(false));
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

    /// Interface plan I4, D-I-11: a composing hint fans out to the room
    /// minus the typist, carries them as the perception subject, is
    /// best-effort, never stamps idle, and stays silent for a DARK actor,
    /// an opt-out, or a missing actor.
    #[test]
    fn composing_fans_out_to_the_room_without_stamping_idle() {
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
        let before = engine.world.session_stat(nick).map(|s| s.last_input);
        engine.world.set_now(5_000);
        let mut out = Outbox::new();
        engine.step(
            Inbound::Composing {
                conn: 2,
                actor: nick,
                active: true,
            },
            &mut out,
            &mut fx,
        );
        assert_eq!(
            engine.world.session_stat(nick).map(|s| s.last_input),
            before,
            "a composing hint is never an idle stamp"
        );
        assert_eq!(out.len(), 1, "{:?}", out.events());
        let (aud, ev) = &out.events()[0];
        assert!(
            matches!(aud, Audience::Room { room, exclude, subject }
                if *room == LIMBO && exclude == &vec![nick] && *subject == Some(nick)),
            "{aud:?}"
        );
        assert!(
            matches!(ev, OutputEvent::Composing { actor, active: true } if actor.name == "Nick"),
            "{ev:?}"
        );
        assert_eq!(out.take_classified()[0].0, DeliveryClass::BestEffort);

        // `active: false` fans out the same way.
        let mut out = Outbox::new();
        engine.step(
            Inbound::Composing {
                conn: 2,
                actor: nick,
                active: false,
            },
            &mut out,
            &mut fx,
        );
        assert!(matches!(
            &out.events()[0].1,
            OutputEvent::Composing { active: false, .. }
        ));

        // An opt-out silences it.
        engine
            .world
            .set_attr(nick, COMPOSING_PREF_ATTR, crate::Attr::text("off", nick))
            .unwrap();
        let mut out = Outbox::new();
        engine.step(
            Inbound::Composing {
                conn: 2,
                actor: nick,
                active: true,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty(), "opted out: {:?}", out.events());
        engine
            .world
            .set_attr(nick, COMPOSING_PREF_ATTR, crate::Attr::text("on", nick))
            .unwrap();

        // A DARK actor never emits it.
        engine.world.set_flag(nick, Flags::DARK, true).unwrap();
        let mut out = Outbox::new();
        engine.step(
            Inbound::Composing {
                conn: 2,
                actor: nick,
                active: true,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty(), "DARK: {:?}", out.events());

        // A vanished actor is dropped, not a panic.
        let mut out = Outbox::new();
        engine.step(
            Inbound::Composing {
                conn: 9,
                actor: ObjId(999),
                active: true,
            },
            &mut out,
            &mut fx,
        );
        assert!(out.is_empty());
        assert!(composing_opted_out(" OFF ") && composing_opted_out("0"));
        assert!(!composing_opted_out("on") && !composing_opted_out(""));
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
