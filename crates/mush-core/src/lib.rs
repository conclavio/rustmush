//! # mush-core
//!
//! The synchronous heart of rustmush: the in-RAM [`World`], the object
//! model (objects, attributes, flags, locks, zones), and the IC game clock.
//!
//! Design ground rules (see `docs/PLAN.md`):
//!
//! - **No I/O, no async, no locks.** `World` is plain data plus methods; at
//!   runtime a single task owns it outright, and in tests it is driven
//!   directly. Nothing in this crate's public API mentions tokio.
//! - **All mutation goes through `World` methods**, which maintain every
//!   inverse index (contents/exits, player names, zone membership), stamp
//!   `modified`, and record dirty ids for the persistence layer.
//! - **Ids are never recycled** ([`obj::ObjId`] is monotonic; destroyed
//!   objects leave tombstones), so a stale dbref can never alias a new
//!   object.
//!
//! On top of the foundation sit the command layer:
//!
//! - [`perm`] — centralized permission checks (`controls`, `can_examine`,
//!   power/role gating); handlers never inline flag math.
//! - [`output`] — [`Audience`]/[`Outbox`] plus view builders emitting
//!   `mush-proto` types **directly** (treaty ruling 1: no parallel event
//!   enum in core).
//! - [`matchit`] — the single classic-precedence name resolver
//!   (`me`/`here`/`#dbref`/`*player`/inventory/location/exits/prefix/
//!   `2nd sword`).
//! - [`cmd`] — [`CommandSpec`]/[`Registry`]/[`dispatch`], implementing the
//!   treaty ruling 5 pipeline (prefix chars → alias stage → HOME → exits →
//!   command table → Huh).
//! - [`commands`] — the core MUSH command set, registered via
//!   [`register_core_commands`].
//! - [`module`] — the merged module API (treaty ruling 4): [`GameModule`]
//!   and [`Registrar`] (commands, alias stages, marker flags, attr
//!   namespaces, ticks, hooks, observers, state slots, panel feeds, query
//!   handlers, job queues, script-api reservations, migrations), module
//!   persistence ops ([`ModuleOp`]), scenes ([`SceneId`], the btech seam),
//!   and the input-context stack.
//!
//! And the async seam (still fully sync in here — the tokio loop lives in
//! `mushd`):
//!
//! - [`engine`] — [`Engine`]/[`Inbound`]/[`Effects`]/[`WorldCommand`]: the
//!   step function the world task drives, connection bookkeeping, and the
//!   typed re-entry path for async completions (treaty ruling 3).
//! - [`sched`] — the pure min-heap [`Scheduler`] behind `@wait` and module
//!   ticks.

pub mod attr;
pub mod behavior;
pub mod clock;
pub mod cmd;
pub mod commands;
pub mod engine;
pub mod flags;
pub mod ledger;
pub mod lock;
pub mod matchit;
pub mod module;
pub mod obj;
pub mod output;
pub mod perm;
pub mod sched;
pub mod world;

pub use attr::{Attr, AttrFlags, AttrMap, AttrValue, UncasedName};
pub use behavior::{
    BINDING_SCHEMA_VERSION, BehaviorBinding, BehaviorCommand, BehaviorCommandHandler, BehaviorId,
    BehaviorIdError, BehaviorRegError, BehaviorResolution, BehaviorSpec, BehaviorTarget,
    CommandReach, DestroyGate, DestroyPolicy, DetachGate, MAX_CARRIED_SCAN,
    resolve_behavior_command,
};
pub use clock::{CivilDate, GameClock, Season, TimeOfDay};
pub use cmd::{
    AliasOutcome, AliasStage, ArgShape, CmdError, CommandFallback, CommandGate, CommandHandler,
    CommandSpec, Ctx, HelpTopic, LookupErr, ParsedArgs, Registry, RegistryError, SceneClass,
    SwitchSet, dispatch,
};
pub use commands::{
    event_subject, ignore_list, is_ignoring, narrative_actor, register_core_commands,
};
pub use engine::{
    ControlMsg, Deferred, DisconnectKind, Effect, Effects, Engine, Inbound, StepOutcome,
    WorldCommand, register_engine_commands,
};
pub use flags::{Flags, Powers, Role};
pub use ledger::{Ledger, LedgerChunk, LedgerEntry, chunk_key, is_log_key};
pub use lock::{LockExpr, LockParseError, LockType, eval_lock, glob_match, parse_lock};
pub use matchit::{MatchErr, match_object};
pub use module::{
    AsyncCompletion, ContextOutcome, EventHook, GameModule, HookPoint, InputContext, JobQueueSpec,
    Migration, ModError, ModuleBootData, ModuleCtx, ModuleOp, ModuleWrite, PanelFeedHandle,
    Perceive, QueryHandler, Registrar, Scene, SceneId, StackedContext, TickCadence, TickHandler,
    WeatherQuery, WorldEvent, WorldEventKind, WorldObserver, WxSnapshot, can_perceive,
    is_concealed, perceives_via,
};
pub use obj::{GOD, LIMBO, Obj, ObjId, ObjKind};
pub use output::{Audience, ConnId, Outbox, name_span, obj_ref, room_view};
pub use sched::{ScheduledKind, Scheduler, TimerId};
pub use world::{CommandSubjectLink, MAX_CONTAINMENT_DEPTH, SessionStat, World, WorldError};
