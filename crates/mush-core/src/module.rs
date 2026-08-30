//! The merged module API (treaty ruling 4): [`GameModule`] + [`Registrar`].
//!
//! Modules are compiled-in crates handed to
//! [`Engine::with_modules`](crate::engine::Engine::with_modules) as
//! `Box<dyn GameModule>` **before** world load. Each module's
//! [`GameModule::register`] runs once against a [`Registrar`], which is a
//! thin, identity-carrying view over the existing
//! [`Registry`](crate::cmd::Registry) — commands and alias stages land in
//! the same tables the core uses (there is no second registry, ever), and
//! everything else (hooks, ticks, observers, panel feeds, …) lands in the
//! registry's module tables.
//!
//! Modules talk to the world **only** through this API:
//!
//! - world access and output via the same [`Ctx`] command handlers get
//!   (hooks/ticks/observers receive one too — [`ModuleCtx`] is an alias);
//! - persistence via [`Ctx::persist`] and [`ModuleOp`], queued on the
//!   outbox's deferred side-channel and drained by the world task into the
//!   current `WriteBatch` (write-through, same atomicity as object writes);
//! - typed state via [`Registrar::state`] slots living on the
//!   [`World`](crate::world::World);
//! - scenes (the BattleTech seam) via [`Ctx::start_scene`] /
//!   [`Ctx::end_scene`] / scene ticks / the per-scene state store and map
//!   blob.
//!
//! Comsys, mail, and jobs are built on this API first (M3) as its proving
//! ground; the btech stub keeps the scene/tick/veto seams green forever.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use mush_proto::{NarrativeKind, OutputEvent, PanelFrameKind, PromptSpec, RichText};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::clock::{Season, TimeOfDay};
use crate::cmd::{
    AliasStage, CmdError, CommandFallback, CommandGate, CommandHandler, CommandSpec, Ctx,
    ParsedArgs, Registry, SwitchSet,
};
use crate::engine::Deferred;
use crate::obj::{GOD, ObjId, ObjKind};
use crate::output::{Audience, Outbox};
use crate::world::World;

/// A module context is the same [`Ctx`] command handlers receive (treaty
/// ruling 4's unification): world access, outbox helpers,
/// [`persist`](Ctx::persist), state slots, and the scene API in one place.
/// Hooks, ticks, observers, and [`GameModule::on_world_loaded`] all get one.
pub type ModuleCtx<'a> = Ctx<'a>;

// ----------------------------------------------------------------------
// Errors
// ----------------------------------------------------------------------

/// Module registration / boot failure. Registration errors are programming
/// errors surfaced at startup — the server does not boot with a broken
/// module set.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModError {
    /// A command name or alias collided with an existing registration.
    #[error("{0}")]
    Registry(#[from] crate::cmd::RegistryError),
    /// A module-table registration collided (attr namespace, marker flag,
    /// panel feed, query handler, tick tag, script api).
    #[error("module {module:?}: {what} {name:?} is already registered by module {owner:?}")]
    Conflict {
        /// The module attempting the registration.
        module: SmolStr,
        /// What kind of registration collided ("attr namespace", …).
        what: &'static str,
        /// The colliding name.
        name: SmolStr,
        /// The module that already owns the name.
        owner: SmolStr,
    },
    /// Free-form module boot error (bad persisted data, …).
    #[error("{0}")]
    Msg(String),
}

impl ModError {
    /// Shorthand for a free-form module error.
    pub fn msg(m: impl Into<String>) -> ModError {
        ModError::Msg(m.into())
    }
}

// ----------------------------------------------------------------------
// Module persistence ops (shared contract with mush-store)
// ----------------------------------------------------------------------

/// One write against a module's persisted namespace.
///
/// Redis mapping (applied by mush-store inside the same MULTI/EXEC as the
/// rest of the command's `WriteBatch`):
///
/// - `JsonSet` → `JSON.SET mod:{ns}:{key} $ <value>`
/// - `Del` → `DEL mod:{ns}:{key}`
/// - `StreamAppend` → `XADD mod:{ns}:{key} MAXLEN ~ {maxlen} * data <json>`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ModuleWrite {
    /// Set the JSON document at `key` to `value` (whole-doc replace).
    JsonSet {
        /// Key within the module's namespace.
        key: SmolStr,
        /// The document.
        value: serde_json::Value,
    },
    /// Delete the document at `key`.
    Del {
        /// Key within the module's namespace.
        key: SmolStr,
    },
    /// Append `value` to the stream at `key`, approximately capped at
    /// `maxlen` entries (ring history: channel scrollback, audit trails).
    StreamAppend {
        /// Stream key within the module's namespace.
        key: SmolStr,
        /// The entry payload.
        value: serde_json::Value,
        /// Approximate maximum retained entries.
        maxlen: u32,
    },
}

/// A namespaced module write, queued via [`Ctx::persist`] and carried into
/// the per-command `WriteBatch` by the world task (write-through: it commits
/// atomically with the same command's object mutations).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleOp {
    /// The owning module's persistence namespace, e.g. `"comsys"`.
    pub ns: SmolStr,
    /// The write itself.
    pub op: ModuleWrite,
}

/// A module's persisted data, loaded by the driver **out-of-band at module
/// load time** (never during play) through the store's read API and handed
/// to [`GameModule::on_world_loaded`]. `mush-core` only defines the shape.
#[derive(Clone, Debug, Default)]
pub struct ModuleBootData {
    /// Every `mod:{ns}:{key}` JSON document: `(key, value)` pairs.
    pub kv: Vec<(SmolStr, serde_json::Value)>,
    /// Named stream tails, oldest-first: stream key → `(entry id, value)`.
    pub streams: HashMap<SmolStr, Vec<(String, serde_json::Value)>>,
}

// ----------------------------------------------------------------------
// Scenes (the btech seam)
// ----------------------------------------------------------------------

/// Identifier for one active scene (per-battle instance, roll-log scene,
/// …). Monotonic per boot; scenes are transient world state whose module
/// data persists via [`ModuleOp`]s.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SceneId(pub u64);

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scene#{}", self.0)
    }
}

/// One active scene: an object set bound to a ruleset-defined kind.
/// WoD uses scenes lightly (roll logs, beat awards); btech binds a hex map
/// and a fast ticker to one.
///
/// **Boot contract**: scenes are transient world state and scene ids
/// restart at 0 every boot, but a crash without [`Ctx::end_scene`] leaves
/// the scene's persisted `scene:{id}:*` documents behind. A module that
/// persists scene state MUST purge stale `scene:*` keys from its namespace
/// in [`GameModule::on_world_loaded`] (queue `Del` ops for them), or the
/// next boot's scene #0 interleaves with the orphan's keys. The btech stub
/// demonstrates the pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    /// The scene's id.
    pub id: SceneId,
    /// Module-defined kind, e.g. `"btech.battle"`.
    pub kind: SmolStr,
    /// The persistence namespace the scene's state/map documents live in —
    /// the module that created the scene (recorded at
    /// [`Ctx::start_scene`]), so scene persistence never depends on *who*
    /// later touches or ends the scene.
    pub ns: SmolStr,
    /// Participating objects (units, characters).
    pub members: BTreeSet<ObjId>,
    /// The persistence key the scene's ruleset-state snapshots are written
    /// under (within `ns`).
    pub ruleset_state_key: SmolStr,
}

// ----------------------------------------------------------------------
// Ticks
// ----------------------------------------------------------------------

/// How often a module tick fires.
///
/// `Fast` may be sub-second: the M1 world loop sleeps until
/// [`Scheduler::next_due`](crate::sched::Scheduler::next_due) (a
/// millisecond deadline), so its granularity already supports sub-second
/// cadences — no busy loop, no fixed frame rate.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TickCadence {
    /// Second-or-slower housekeeping: weather, regen, upkeep. Scheduled
    /// at engine construction and re-armed after every fire.
    Coarse(Duration),
    /// Sub-second allowed; reserved for real-time combat. Scheduled and
    /// re-armed like `Coarse`.
    Fast(Duration),
    /// A per-scene ticker: registered here, **started on demand** via
    /// [`Ctx::scene_tick_start`] (which supplies the period) and removed
    /// dynamically via [`Ctx::scene_tick_stop`] or automatically when its
    /// scene ends.
    Scene,
}

/// A module tick implementation. `scene` is `Some` for [`TickCadence::Scene`]
/// firings, `None` for periodic (`Coarse`/`Fast`) ones.
pub trait TickHandler: Send + Sync {
    /// One firing. Runs synchronously in the world task with a full
    /// [`ModuleCtx`] (actor is `#1`; `ctx.module` is the owning module).
    fn tick(&self, ctx: &mut Ctx<'_>, tag: &str, scene: Option<SceneId>);
}

impl<F> TickHandler for F
where
    F: Fn(&mut Ctx<'_>, &str, Option<SceneId>) + Send + Sync,
{
    fn tick(&self, ctx: &mut Ctx<'_>, tag: &str, scene: Option<SceneId>) {
        self(ctx, tag, scene);
    }
}

// ----------------------------------------------------------------------
// Hooks
// ----------------------------------------------------------------------

/// Interception points in core flows. Which [`EventHook`] method fires is
/// selected by the point the hook was registered under.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum HookPoint {
    /// Veto-capable: consulted by exit traversal, `home`, `@teleport`, and
    /// `@dig/teleport` **before** the move happens (exactly the moves that
    /// emit [`WorldEvent::Moved`]). An `Err(reason)` blocks the move and
    /// shows the reason to the mover.
    MoveAttempt,
    /// Append lines to a rendered room view (weather ambience, scene
    /// banners). Fires wherever a room render is shown: `look`, movement
    /// arrival, `@teleport`.
    LookRendered,
    /// Read-only per-observer perception gate (the M11 projection seam):
    /// consulted wherever the output layer decides whether one observer may
    /// perceive one subject — the `look`/`examine` contents filters and the
    /// live `Audience::Room` fan-out. First `Conceal` wins; with no hook
    /// registered the gate is a free pass (behavior byte-for-byte identical
    /// to a game with no concealment ruleset). See [`can_perceive`].
    Perceive,
    /// An actor's session became live (first connection attached).
    Connected,
    /// An actor's session fully ended (clean quit or linkdead grace
    /// expiry).
    Disconnected,
    /// A scene was started via [`Ctx::start_scene`].
    SceneStarted,
    /// A scene is ending via [`Ctx::end_scene`] (fires while the scene is
    /// still queryable).
    SceneEnded,
}

/// A [`EventHook::perceives`] verdict for one `(observer, subject)` pair.
///
/// The gate is *first-conceal-wins*: a hook returns [`Perceive::Conceal`] to
/// hide the subject from that observer, or [`Perceive::Abstain`] (the
/// default) to leave the decision to the other hooks / the base rules. A
/// subject is perceived unless some hook actively conceals it, so a game with
/// no perception hook registered never hides anyone (see [`can_perceive`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Perceive {
    /// This hook takes no position — defer to the remaining hooks.
    #[default]
    Abstain,
    /// Hide `subject` from `observer`.
    Conceal,
}

/// A module hook. Default methods are no-ops; a module implements only the
/// points it registers for (registering under a point whose method is left
/// defaulted is legal and does nothing).
#[allow(unused_variables)]
pub trait EventHook: Send + Sync {
    /// Veto a movement attempt. `exit` is `None` for `@teleport`. Return
    /// `Err(reason)` to block; the reason is shown to the mover.
    fn move_attempt(
        &self,
        w: &mut World,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        Ok(())
    }

    /// Extra lines appended after a room render for `looker`.
    fn look_rendered(&self, w: &World, looker: ObjId, room: ObjId) -> Vec<RichText> {
        Vec::new()
    }

    /// Whether `observer` should be prevented from perceiving `subject`
    /// (the M11 per-observer projection gate). Read-only by contract — the
    /// same `&World` is shared by the look path and the live fan-out, so the
    /// verdict must be a cheap lookup, never a mutation (no rolling, no cache
    /// writes; a ruleset rolls and caches in its mutable paths and only reads
    /// the cache here). Default [`Perceive::Abstain`] leaves the subject
    /// visible. See [`can_perceive`].
    fn perceives(&self, w: &World, observer: ObjId, subject: ObjId) -> Perceive {
        Perceive::Abstain
    }

    /// Whether `subject` is under an *active* concealment effect (e.g. WoD
    /// Obfuscate) — the ruleset-agnostic **notability** signal, distinct from
    /// [`perceives`](EventHook::perceives) (which is the per-observer
    /// projection). Read-only by the same contract as `perceives`: a cheap
    /// cache lookup, never a mutation. Default `false` — with no concealment
    /// ruleset a subject is never concealed. See [`is_concealed`].
    fn is_concealed(&self, w: &World, subject: ObjId) -> bool {
        false
    }

    /// An actor connected (session became live).
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {}

    /// An actor fully disconnected.
    fn disconnected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {}

    /// A scene started.
    fn scene_started(&self, ctx: &mut Ctx<'_>, scene: SceneId) {}

    /// A scene is ending (still queryable during the call).
    fn scene_ended(&self, ctx: &mut Ctx<'_>, scene: SceneId) {}
}

/// One registered hook: the implementing module plus the handler.
#[derive(Clone)]
pub struct HookReg {
    /// The registering module.
    pub module: SmolStr,
    /// The handler.
    pub hook: Arc<dyn EventHook>,
}

impl fmt::Debug for HookReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HookReg({})", self.module)
    }
}

// ----------------------------------------------------------------------
// World events (observers)
// ----------------------------------------------------------------------

/// What kinds of world events a module can subscribe to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WorldEventKind {
    /// An object moved between locations.
    Moved,
    /// Room speech of one specific flavor (`Say`, `Pose`, `SemiPose`,
    /// `Emit`) — subscribe once per flavor you care about.
    Said {
        /// The speech flavor to observe.
        kind: NarrativeKind,
    },
    /// An object was created.
    ObjCreated,
    /// An object was destroyed (`@destroy`; the husk is still resolvable).
    ObjDestroyed,
    /// An actor's session became live.
    Connected,
    /// An actor's session fully ended.
    Disconnected,
    /// An attribute was set or cleared on an object (the
    /// `@set <obj>/<attr>=…` path). Observers filter by the carried name —
    /// mush-script recompiles an object's scripts when a `script:*`
    /// attribute changes.
    AttrSet,
    /// A supernatural power was used and perceived (the M14 perception
    /// director's informant tie-in): a Discipline / spell / Gift whose
    /// per-observer projection has already resolved which onlookers noticed
    /// it. Observers file intel for the carried witness set — no re-gate.
    PowerUsed,
}

/// A world mutation, delivered to subscribed observers **post-mutation** in
/// the world task. Emitted by the dispatch layer's shared helpers — never
/// by [`World`] itself, which stays observer-free.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    /// `who` moved from `from` to `to` (exit traversal, `home`,
    /// `@teleport`, `@dig/teleport` — the same moves `MoveAttempt` can
    /// veto).
    Moved {
        /// The moved object.
        who: ObjId,
        /// Where it was (if anywhere).
        from: Option<ObjId>,
        /// Where it is now.
        to: ObjId,
    },
    /// `who` spoke in `room`.
    Said {
        /// The speaker.
        who: ObjId,
        /// The room spoken in.
        room: ObjId,
        /// Speech flavor (say/pose/semipose/emit).
        kind: NarrativeKind,
        /// The spoken text (plain).
        text: String,
    },
    /// A new object exists.
    ObjCreated {
        /// The new object.
        obj: ObjId,
        /// Its kind.
        kind: ObjKind,
        /// Who created it.
        creator: ObjId,
    },
    /// An object was destroyed (marked `GOING`; husk still resolvable).
    ObjDestroyed {
        /// The destroyed object.
        obj: ObjId,
    },
    /// An actor's session became live.
    Connected {
        /// The actor.
        who: ObjId,
    },
    /// An actor's session fully ended.
    Disconnected {
        /// The actor.
        who: ObjId,
    },
    /// An attribute was set or cleared on `obj` via the `@set` path.
    AttrSet {
        /// The object whose attribute changed.
        obj: ObjId,
        /// The attribute name, original casing (`"script:command:greet"`).
        name: String,
        /// Who set it.
        setter: ObjId,
    },
    /// `who` used a supernatural power in `room`, already projected
    /// per-observer by the M14 perception director. `witnesses` is the
    /// director-resolved set of **NPC** onlookers who *perceived* it — the
    /// event exists to drive the informant loop, which files intel for exactly
    /// these with **no** perception re-gate (re-rolling would double-count what
    /// the director already delivered). `detail` is a short human description
    /// of the act.
    PowerUsed {
        /// The power's user.
        who: ObjId,
        /// The room it happened in.
        room: ObjId,
        /// The NPC onlookers who perceived it (director-resolved winners).
        witnesses: Vec<ObjId>,
        /// A short description of the act ("used a Discipline (dominate)").
        detail: String,
    },
}

impl WorldEvent {
    /// The subscription kind this event matches.
    #[must_use]
    pub fn kind(&self) -> WorldEventKind {
        match self {
            WorldEvent::Moved { .. } => WorldEventKind::Moved,
            WorldEvent::Said { kind, .. } => WorldEventKind::Said { kind: *kind },
            WorldEvent::ObjCreated { .. } => WorldEventKind::ObjCreated,
            WorldEvent::ObjDestroyed { .. } => WorldEventKind::ObjDestroyed,
            WorldEvent::Connected { .. } => WorldEventKind::Connected,
            WorldEvent::Disconnected { .. } => WorldEventKind::Disconnected,
            WorldEvent::AttrSet { .. } => WorldEventKind::AttrSet,
            WorldEvent::PowerUsed { .. } => WorldEventKind::PowerUsed,
        }
    }

    /// The event's subject — used as the `actor` of observer contexts.
    #[must_use]
    pub fn subject(&self) -> ObjId {
        match self {
            WorldEvent::Moved { who, .. }
            | WorldEvent::Said { who, .. }
            | WorldEvent::Connected { who }
            | WorldEvent::Disconnected { who }
            | WorldEvent::PowerUsed { who, .. } => *who,
            WorldEvent::ObjCreated { creator, .. } => *creator,
            WorldEvent::ObjDestroyed { obj } | WorldEvent::AttrSet { obj, .. } => *obj,
        }
    }
}

/// A synchronous world-event observer, invoked post-mutation in the world
/// task. Closures with the same shape register directly.
pub trait WorldObserver: Send + Sync {
    /// Handle one event.
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent);
}

impl<F> WorldObserver for F
where
    F: Fn(&mut Ctx<'_>, &WorldEvent) + Send + Sync,
{
    fn on_event(&self, ctx: &mut Ctx<'_>, ev: &WorldEvent) {
        self(ctx, ev);
    }
}

// ----------------------------------------------------------------------
// Panels, queries, jobs, script APIs, migrations
// ----------------------------------------------------------------------

/// Handle to a registered panel feed. Emits
/// [`OutputEvent::Panel`] frames through the outbox; the web client routes
/// them by feed id, the ANSI renderer falls back to module text renderers
/// (M4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFeedHandle {
    feed: SmolStr,
}

impl PanelFeedHandle {
    /// The feed id (e.g. `"btech.map"`).
    #[must_use]
    pub fn feed(&self) -> &str {
        &self.feed
    }

    /// Emit a full-state keyframe to the acting player.
    pub fn keyframe(&self, ctx: &mut Ctx<'_>, data: serde_json::Value) {
        let aud = Audience::Actor(ctx.actor);
        self.keyframe_to(ctx, aud, data);
    }

    /// Emit a full-state keyframe to an explicit audience.
    pub fn keyframe_to(&self, ctx: &mut Ctx<'_>, aud: Audience, data: serde_json::Value) {
        ctx.out.push(
            aud,
            OutputEvent::Panel {
                feed: self.feed.clone(),
                kind: PanelFrameKind::Keyframe,
                data,
            },
        );
    }

    /// Emit an incremental delta to the acting player.
    pub fn delta(&self, ctx: &mut Ctx<'_>, data: serde_json::Value) {
        let aud = Audience::Actor(ctx.actor);
        self.delta_to(ctx, aud, data);
    }

    /// Emit an incremental delta to an explicit audience.
    pub fn delta_to(&self, ctx: &mut Ctx<'_>, aud: Audience, data: serde_json::Value) {
        ctx.out.push(
            aud,
            OutputEvent::Panel {
                feed: self.feed.clone(),
                kind: PanelFrameKind::Delta,
                data,
            },
        );
    }
}

/// One weather observation for a room's zone — the reserved `WeatherQuery`
/// payload (design doc 04 §10.5). Fields are deliberately few and physical:
/// the weather module (M5) fills them from its state machine; consumers
/// (WoD perception modifiers, btech to-hit modifiers) read, never mutate.
#[derive(Clone, Debug, PartialEq)]
pub struct WxSnapshot {
    /// The machine state name, e.g. `"clear"`, `"rain"`, `"snow"`.
    pub state: SmolStr,
    /// Visibility fraction, `0.0` (whiteout) ..= `1.0` (clear).
    pub visibility: f32,
    /// Precipitation intensity, `0.0` ..= `1.0`.
    pub precipitation: f32,
    /// Wind speed, m/s.
    pub wind: f32,
    /// Temperature, °C.
    pub temperature: f32,
}

/// The reserved environment-query seam (PLAN's btech seam list; design doc
/// 04 §10.5): rulesets read weather snapshots through this trait, never
/// through the weather module directly. The weather module (M5) registers
/// the single provider via [`Registrar::weather_query`]; consumers reach it
/// via [`Registry::weather_query`]. `None` means no data (indoors, no
/// provider, unzoned room).
pub trait WeatherQuery: Send + Sync {
    /// The current weather at `room` (resolved through its zone).
    fn current(&self, w: &World, room: ObjId) -> Option<WxSnapshot>;
}

/// A read-only query handler for web `Req` hydration (mail lists, channel
/// backlogs, sheet reads), invoked by the engine's
/// [`Inbound::Query`](crate::engine::Inbound::Query) handling (M4).
///
/// The handler answers with a full [`OutputEvent`] (typically a
/// [`OutputEvent::Panel`] keyframe or a `Who` listing); the engine wraps it
/// in [`OutputEvent::Reply`] with the request's correlation id and
/// addresses it to the requesting **connection** only. An `Err` is a
/// player-readable refusal; the engine delivers it as a `Reply` wrapping a
/// `Sys` line so the client can still correlate it.
///
/// Handlers are **read-only by contract**: they get `&World` and must not
/// queue persistence — a query must never mutate (that is why `@mail`
/// *reading* stays a `Line` command: it marks messages read).
pub trait QueryHandler: Send + Sync {
    /// Answer one query for `actor`.
    fn query(
        &self,
        w: &World,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText>;
}

/// Typed re-entry of a module's deferred async work
/// ([`Effect::ModuleAsync`](crate::engine::Effect::ModuleAsync) →
/// [`WorldCommand::ModuleAsyncDone`](crate::engine::WorldCommand::ModuleAsyncDone),
/// treaty ruling 3). The driving loop runs the real future off the world
/// loop (it holds the service handle — `mush-core` never does) and re-enters
/// with the result `payload`; the engine routes it here, in the world task,
/// with a full [`Ctx`] carrying the owning module's identity.
///
/// **Re-validate against current world state**: the world may have changed
/// while the async work ran (the room was `@destroy`ed, the actor
/// disconnected). Persistence goes through [`Ctx::persist`] as usual.
pub trait AsyncCompletion: Send + Sync {
    /// Apply one completion. `payload` is this module's own JSON (whatever it
    /// serialized into the originating `Effect::ModuleAsync`); `correlation`
    /// echoes the effect's id; `ctx.actor` is the triggering actor.
    fn on_complete(&self, ctx: &mut Ctx<'_>, correlation: u64, payload: &str);
}

/// A module job queue declaration (chargen approval, XP spends, +requests).
/// **Registry only in M3** — the jobs module consumes these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobQueueSpec {
    /// Queue id, e.g. `"xp"`, `"chargen"`.
    pub name: SmolStr,
    /// Human title shown in staff tooling.
    pub title: String,
    /// Minimum staff role able to work the queue.
    pub min_role: crate::flags::Role,
}

impl Default for JobQueueSpec {
    /// An unnamed queue workable by Wizards.
    fn default() -> JobQueueSpec {
        JobQueueSpec {
            name: SmolStr::default(),
            title: String::new(),
            min_role: crate::flags::Role::Wizard,
        }
    }
}

/// One versioned migration over a module's persisted documents, collected
/// at registration for the **driver** to run against the store (with the
/// mandatory BGSAVE-before-migrate). `mush-core` never runs these itself.
///
/// `apply` receives the document's **key** (within the namespace) alongside
/// its value: real namespaces are heterogeneous (`msg:*` vs `box:*` vs
/// `next_id`), so a migration reshaping one document family must be able to
/// route on the key rather than shape-sniffing the value. **Streams are
/// explicitly out of scope**: migrations transform kv documents only —
/// a module that must reshape stream history does it lazily at boot
/// (tolerate-old-entries) or via a dedicated offline tool.
#[derive(Clone)]
pub struct Migration {
    /// Target schema version this migration produces.
    pub version: u32,
    /// What it does, for logs.
    pub description: String,
    /// Transform one persisted document (`(key, value)`) from the previous
    /// version.
    #[allow(clippy::type_complexity)]
    pub apply:
        Arc<dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>,
}

impl fmt::Debug for Migration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Migration(v{}: {})", self.version, self.description)
    }
}

// ----------------------------------------------------------------------
// Input context stack
// ----------------------------------------------------------------------

/// What an [`InputContext`] did with a fed line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextOutcome {
    /// The line was consumed; the context stays on the stack.
    Consumed,
    /// The context is done; pop it (the previous prompt is restored).
    Pop,
    /// Pop the context, then dispatch this line through the normal
    /// pipeline (e.g. mail compose's final `send` turning into a command).
    PopAndRun(String),
}

/// A pushed input mode capturing an actor's raw lines before normal
/// dispatch — mail compose, chargen wizards, the btech unit console.
///
/// The stack is checked **before everything else** in dispatch (even
/// prefix characters), with one hard escape: a lone `~q` always pops the
/// top context with an abort message, so no context can trap a player.
pub trait InputContext: Send {
    /// The prompt shown while this context is on top.
    fn prompt(&self, w: &World) -> PromptSpec;

    /// Feed one input line. Runs in the world task with a full [`Ctx`]
    /// (empty args/switches; `ctx.module` is the module identity the
    /// context was pushed under — so a module's context keeps its
    /// attr-namespace and scene privileges across every fed line).
    fn feed(&mut self, ctx: &mut Ctx<'_>, line: &str) -> ContextOutcome;
}

/// One stacked input context plus the module identity it was pushed under
/// (fed back into the [`Ctx`] for every line it consumes).
pub struct StackedContext {
    /// The pushing module's identity (`None` for core-pushed contexts).
    pub module: Option<SmolStr>,
    /// The context itself.
    pub cx: Box<dyn InputContext>,
}

/// Per-actor input-context stacks, owned by the
/// [`World`](crate::world::World).
///
/// **Transient parser state**: contexts are trait objects and cannot be
/// cloned — cloning a `World` yields empty stacks (a cloned world is a
/// snapshot for inspection, not a live session).
#[derive(Default)]
pub struct InputStacks {
    stacks: HashMap<ObjId, Vec<StackedContext>>,
}

impl Clone for InputStacks {
    fn clone(&self) -> InputStacks {
        InputStacks::default()
    }
}

impl fmt::Debug for InputStacks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InputStacks({} actors)", self.stacks.len())
    }
}

impl InputStacks {
    /// Stack depth for an actor.
    #[must_use]
    pub fn depth(&self, actor: ObjId) -> usize {
        self.stacks.get(&actor).map_or(0, Vec::len)
    }

    /// Push a context (tagged with the pushing module's identity) on top of
    /// an actor's stack.
    pub fn push(&mut self, actor: ObjId, module: Option<SmolStr>, cx: Box<dyn InputContext>) {
        self.stacks
            .entry(actor)
            .or_default()
            .push(StackedContext { module, cx });
    }

    /// Pop the top context, if any.
    pub fn pop(&mut self, actor: ObjId) -> Option<StackedContext> {
        let stack = self.stacks.get_mut(&actor)?;
        let cx = stack.pop();
        if stack.is_empty() {
            self.stacks.remove(&actor);
        }
        cx
    }

    /// The top context, if any.
    #[must_use]
    pub fn top(&self, actor: ObjId) -> Option<&dyn InputContext> {
        self.stacks
            .get(&actor)
            .and_then(|s| s.last())
            .map(|s| s.cx.as_ref())
    }

    /// Re-insert a context at `index` (clamped) — used by dispatch to put
    /// a fed context back beneath anything it pushed while feeding.
    pub fn insert_at(&mut self, actor: ObjId, index: usize, cx: StackedContext) {
        let stack = self.stacks.entry(actor).or_default();
        let index = index.min(stack.len());
        stack.insert(index, cx);
    }

    /// Drop an actor's whole stack (destroyed actors).
    pub fn clear(&mut self, actor: ObjId) {
        self.stacks.remove(&actor);
    }
}

// ----------------------------------------------------------------------
// Typed module state slots
// ----------------------------------------------------------------------

type StateCloneFn = fn(&(dyn Any + Send)) -> Box<dyn Any + Send>;

fn clone_state<T: Any + Send + Clone>(v: &(dyn Any + Send)) -> Box<dyn Any + Send> {
    Box::new(
        v.downcast_ref::<T>()
            .expect("state slot type invariant")
            .clone(),
    )
}

/// One state slot awaiting installation on the world (collected by
/// [`Registrar::state`], applied by
/// [`Engine::with_modules`](crate::engine::Engine::with_modules)).
pub struct StateInit {
    type_id: TypeId,
    /// The module that declared the slot (conflict detection: two modules
    /// declaring the same `TypeId` is a boot error, not a silent replace).
    module: SmolStr,
    value: Box<dyn Any + Send>,
    clone_fn: StateCloneFn,
}

impl StateInit {
    /// The `TypeId` of the declared slot.
    #[must_use]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// The module that declared it.
    #[must_use]
    pub fn module(&self) -> &SmolStr {
        &self.module
    }
}

impl fmt::Debug for StateInit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StateInit({:?})", self.type_id)
    }
}

struct StateSlot {
    value: Box<dyn Any + Send>,
    clone_fn: StateCloneFn,
}

/// Typed module state living on the [`World`](crate::world::World): an
/// anymap over `TypeId`. Slots are declared via [`Registrar::state`] (or
/// inserted at boot) and accessed through [`Ctx::state`] /
/// [`Ctx::state_mut`].
///
/// State slots are **in-RAM working state** — they are not persisted; a
/// module persists what matters via [`Ctx::persist`].
#[derive(Default)]
pub struct ModuleStateMap {
    slots: HashMap<TypeId, StateSlot>,
}

impl Clone for ModuleStateMap {
    fn clone(&self) -> ModuleStateMap {
        ModuleStateMap {
            slots: self
                .slots
                .iter()
                .map(|(&k, v)| {
                    (
                        k,
                        StateSlot {
                            value: (v.clone_fn)(v.value.as_ref()),
                            clone_fn: v.clone_fn,
                        },
                    )
                })
                .collect(),
        }
    }
}

impl fmt::Debug for ModuleStateMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ModuleStateMap({} slots)", self.slots.len())
    }
}

impl ModuleStateMap {
    /// Install (or replace) the slot for `T`.
    pub fn insert<T: Any + Send + Clone>(&mut self, value: T) {
        self.slots.insert(
            TypeId::of::<T>(),
            StateSlot {
                value: Box::new(value),
                clone_fn: clone_state::<T>,
            },
        );
    }

    /// Install a collected [`StateInit`].
    pub fn apply(&mut self, init: StateInit) {
        self.slots.insert(
            init.type_id,
            StateSlot {
                value: init.value,
                clone_fn: init.clone_fn,
            },
        );
    }

    /// Read the slot for `T`.
    #[must_use]
    pub fn get<T: Any + Send>(&self) -> Option<&T> {
        self.slots
            .get(&TypeId::of::<T>())
            .and_then(|s| s.value.downcast_ref::<T>())
    }

    /// Mutably access the slot for `T`.
    pub fn get_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        self.slots
            .get_mut(&TypeId::of::<T>())
            .and_then(|s| s.value.downcast_mut::<T>())
    }
}

// ----------------------------------------------------------------------
// Registration storage (embedded in Registry)
// ----------------------------------------------------------------------

/// One registered module tick.
#[derive(Clone)]
pub struct TickReg {
    /// The registering module.
    pub module: SmolStr,
    /// The module-chosen tag ("weather", "battle", …).
    pub tag: SmolStr,
    /// How often it fires (or `Scene`).
    pub cadence: TickCadence,
    /// The handler.
    pub handler: Arc<dyn TickHandler>,
}

impl fmt::Debug for TickReg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TickReg({}:{} {:?})",
            self.module, self.tag, self.cadence
        )
    }
}

struct ObserverReg {
    kind: WorldEventKind,
    module: SmolStr,
    observer: Arc<dyn WorldObserver>,
}

/// The module-facing registration tables carried by the
/// [`Registry`](crate::cmd::Registry) — one registry, extended (treaty
/// ruling 4), never a second one.
#[derive(Default)]
pub struct ModuleTables {
    /// attr namespace (lowercased, no trailing `:`) → owning module.
    attr_ns: HashMap<SmolStr, SmolStr>,
    /// marker flag name → owning module (few entries; matched
    /// case-insensitively, canonical case preserved).
    markers: Vec<(SmolStr, SmolStr)>,
    hooks: HashMap<HookPoint, Vec<HookReg>>,
    observers: Vec<ObserverReg>,
    ticks: Vec<TickReg>,
    panel_feeds: Vec<(SmolStr, SmolStr)>,
    query_handlers: HashMap<SmolStr, (SmolStr, Arc<dyn QueryHandler>)>,
    job_queues: Vec<JobQueueSpec>,
    script_apis: Vec<(SmolStr, SmolStr)>,
    migrations: HashMap<SmolStr, Vec<Migration>>,
    /// migration namespace → the one module that owns its migration set.
    migration_owners: HashMap<SmolStr, SmolStr>,
    /// The single registered weather provider (the reserved btech seam).
    weather: Option<(SmolStr, Arc<dyn WeatherQuery>)>,
    /// module name → its async-completion handler (the LLM re-entry seam).
    async_completions: HashMap<SmolStr, Arc<dyn AsyncCompletion>>,
    state_inits: Vec<StateInit>,
}

impl fmt::Debug for ModuleTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ModuleTables(ns:{} markers:{} ticks:{} observers:{})",
            self.attr_ns.len(),
            self.markers.len(),
            self.ticks.len(),
            self.observers.len()
        )
    }
}

impl Registry {
    /// The module that owns attribute namespace `ns` (`"wod"` for
    /// `wod:*`), if any. Lookup is case-insensitive.
    #[must_use]
    pub fn attr_ns_owner(&self, ns: &str) -> Option<&SmolStr> {
        self.mods.attr_ns.get(ns.to_lowercase().as_str())
    }

    /// Every module-owned attribute-namespace key (lowercased, no trailing
    /// `:`). Lets a module snapshot the ownership set at boot and enforce it
    /// later without a live `Registry` (mush-script writes attrs as the
    /// script owner and must refuse module namespaces off-loop).
    #[must_use]
    pub fn module_attr_namespaces(&self) -> Vec<SmolStr> {
        self.mods.attr_ns.keys().cloned().collect()
    }

    /// Whether `who` (acting on behalf of `module`, when set) may write the
    /// attribute `name`: unnamespaced attrs are always fine, module-owned
    /// namespaces admit the owning module and Wizards+ only.
    #[must_use]
    pub fn may_write_attr(&self, w: &World, who: ObjId, module: Option<&str>, name: &str) -> bool {
        let Some((ns, _)) = name.split_once(':') else {
            return true;
        };
        let Some(owner) = self.attr_ns_owner(ns) else {
            return true;
        };
        module == Some(owner.as_str()) || crate::perm::has_role(w, who, crate::flags::Role::Wizard)
    }

    /// The canonical name and owning module of a registered marker flag,
    /// matched case-insensitively.
    #[must_use]
    pub fn marker_owner(&self, name: &str) -> Option<(&SmolStr, &SmolStr)> {
        self.mods
            .markers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(n, m)| (n, m))
    }

    /// The hooks registered under `point`, in registration order.
    #[must_use]
    pub fn hooks(&self, point: HookPoint) -> &[HookReg] {
        self.mods.hooks.get(&point).map_or(&[], Vec::as_slice)
    }

    /// The observers subscribed to `ev`'s kind, in registration order.
    #[must_use]
    pub fn observers_matching(&self, ev: &WorldEvent) -> Vec<(SmolStr, Arc<dyn WorldObserver>)> {
        let kind = ev.kind();
        self.mods
            .observers
            .iter()
            .filter(|o| o.kind == kind)
            .map(|o| (o.module.clone(), Arc::clone(&o.observer)))
            .collect()
    }

    /// The tick registered by `module` under `tag`, if any.
    #[must_use]
    pub fn tick_reg(&self, module: &str, tag: &str) -> Option<&TickReg> {
        self.mods
            .ticks
            .iter()
            .find(|t| t.module == module && t.tag == tag)
    }

    /// All registered module ticks (registration order).
    #[must_use]
    pub fn module_ticks(&self) -> &[TickReg] {
        &self.mods.ticks
    }

    /// All registered panel feed ids with their owning modules.
    #[must_use]
    pub fn panel_feeds(&self) -> &[(SmolStr, SmolStr)] {
        &self.mods.panel_feeds
    }

    /// The query handler registered under `name`, with its owning module.
    #[must_use]
    pub fn query_handler(&self, name: &str) -> Option<(&SmolStr, Arc<dyn QueryHandler>)> {
        self.mods
            .query_handlers
            .get(name)
            .map(|(m, h)| (m, Arc::clone(h)))
    }

    /// All registered job queues.
    #[must_use]
    pub fn job_queues(&self) -> &[JobQueueSpec] {
        &self.mods.job_queues
    }

    /// All reserved script API names with their owning modules (`mush::<name>`).
    #[must_use]
    pub fn script_apis(&self) -> &[(SmolStr, SmolStr)] {
        &self.mods.script_apis
    }

    /// Collected migrations by namespace, for the driver to run.
    #[must_use]
    pub fn migrations(&self) -> &HashMap<SmolStr, Vec<Migration>> {
        &self.mods.migrations
    }

    /// The registered weather provider (the reserved `WeatherQuery` seam),
    /// with its owning module, if one is loaded.
    #[must_use]
    pub fn weather_query(&self) -> Option<(&SmolStr, Arc<dyn WeatherQuery>)> {
        self.mods.weather.as_ref().map(|(m, p)| (m, Arc::clone(p)))
    }

    /// The async-completion handler registered by `module`, if any (the
    /// engine routes [`WorldCommand::ModuleAsyncDone`](crate::engine::WorldCommand::ModuleAsyncDone)
    /// here).
    #[must_use]
    pub fn async_completion(&self, module: &str) -> Option<Arc<dyn AsyncCompletion>> {
        self.mods.async_completions.get(module).map(Arc::clone)
    }

    /// Drain the collected state-slot inits (the engine installs them on
    /// the world at construction).
    pub fn take_state_inits(&mut self) -> Vec<StateInit> {
        std::mem::take(&mut self.mods.state_inits)
    }

    pub(crate) fn tables_mut(&mut self) -> &mut ModuleTables {
        &mut self.mods
    }
}

// ----------------------------------------------------------------------
// GameModule + Registrar
// ----------------------------------------------------------------------

/// A compiled-in game module (comsys, mail, jobs, weather, rulesets).
/// Registered as `Box<dyn GameModule>` **before** world load via
/// [`Engine::with_modules`](crate::engine::Engine::with_modules).
pub trait GameModule: Send {
    /// Stable module id — also the persistence namespace and the identity
    /// commands registered here carry (`ctx.module`).
    fn name(&self) -> &'static str;

    /// Register everything (commands, hooks, ticks, …). Runs once at
    /// engine construction, before any command dispatches.
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError>;

    /// Called once after the world is loaded, with this module's persisted
    /// data (loaded by the driver out-of-band via the store read API).
    /// Default: nothing.
    fn on_world_loaded(
        &mut self,
        _ctx: &mut ModuleCtx<'_>,
        _data: ModuleBootData,
    ) -> Result<(), ModError> {
        Ok(())
    }

    /// Which persisted streams the driver should load into this module's
    /// [`ModuleBootData::streams`], given the module's kv documents (as the
    /// driver just listed them): `(stream key, newest-N to load)` pairs.
    /// Pure over `kv` — called after the kv listing and before
    /// [`GameModule::on_world_loaded`], so a module whose stream keys are
    /// derived from its docs (comsys `hist:{id}` per `chan:{id}`) can name
    /// them without the driver knowing its layout. Default: none.
    fn boot_stream_requests(&self, _kv: &[(SmolStr, serde_json::Value)]) -> Vec<(SmolStr, u32)> {
        Vec::new()
    }
}

/// The registration surface handed to [`GameModule::register`]: an
/// identity-carrying view over the one [`Registry`]. Commands and alias
/// stages go straight into the existing tables; everything else into the
/// registry's [`ModuleTables`].
pub struct Registrar<'a> {
    module: SmolStr,
    reg: &'a mut Registry,
}

impl<'a> Registrar<'a> {
    /// A registrar acting as `module` over `reg`.
    pub fn new(reg: &'a mut Registry, module: &str) -> Registrar<'a> {
        Registrar {
            module: SmolStr::new(module),
            reg,
        }
    }

    /// The registering module's id.
    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Register a command. Identical to core registration except the entry
    /// is tagged with this module's identity (`ctx.module` in its handler;
    /// used by attr-namespace enforcement).
    pub fn command(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), ModError> {
        self.reg
            .register_tagged(spec, handler, Some(self.module.clone()))?;
        Ok(())
    }

    /// Append an alias-stage hook (the comsys seam; runs in registration
    /// order during dispatch).
    pub fn alias_stage(&mut self, stage: Arc<dyn AliasStage>) {
        self.reg.add_alias_stage(stage);
    }

    /// Append a post-command-table [`CommandFallback`] (object
    /// `$`-commands — the mush-script seam; runs after built-ins/exits,
    /// before `Huh?`, in registration order).
    pub fn command_fallback(&mut self, fallback: Arc<dyn CommandFallback>) {
        self.reg.add_command_fallback(fallback);
    }

    /// Append a pre-dispatch [`CommandGate`] (the onboarding gate — runs
    /// before the alias stage and `home`/exits/commands, in registration
    /// order, so it can gate channel broadcast and movement too). A module
    /// registers one to restrict what an actor may run based on live world
    /// state (e.g. an unapproved character held to the chargen command set).
    pub fn command_gate(&mut self, gate: Arc<dyn CommandGate>) {
        self.reg.add_command_gate(gate);
    }

    /// Register a marker flag (e.g. `"wod:kindred"`): settable via
    /// `@set <target>=[!]<name>`, shown by `examine`, matchable by
    /// `flag^` locks.
    pub fn marker_flag(&mut self, name: &str) -> Result<(), ModError> {
        if let Some((canonical, owner)) = self.reg.marker_owner(name) {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "marker flag",
                name: canonical.clone(),
                owner: owner.clone(),
            });
        }
        self.reg
            .tables_mut()
            .markers
            .push((SmolStr::new(name), self.module.clone()));
        Ok(())
    }

    /// Claim an attribute namespace (`"wod"` claims `wod:*`): `@set` then
    /// admits only this module's own commands and Wizards+.
    pub fn attr_namespace(&mut self, ns: &str) -> Result<(), ModError> {
        let key = SmolStr::new(ns.trim_end_matches(':').to_lowercase());
        if let Some(owner) = self.reg.attr_ns_owner(&key) {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "attr namespace",
                name: key,
                owner: owner.clone(),
            });
        }
        self.reg
            .tables_mut()
            .attr_ns
            .insert(key, self.module.clone());
        Ok(())
    }

    /// Register a tick. `Coarse`/`Fast` ticks are armed at engine
    /// construction and re-armed after every fire; `Scene` ticks are
    /// started per scene via [`Ctx::scene_tick_start`]. Tags are unique
    /// per module.
    pub fn tick(
        &mut self,
        tag: &str,
        cadence: TickCadence,
        handler: Arc<dyn TickHandler>,
    ) -> Result<(), ModError> {
        if self.reg.tick_reg(&self.module, tag).is_some() {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "tick tag",
                name: SmolStr::new(tag),
                owner: self.module.clone(),
            });
        }
        self.reg.tables_mut().ticks.push(TickReg {
            module: self.module.clone(),
            tag: SmolStr::new(tag),
            cadence,
            handler,
        });
        Ok(())
    }

    /// Register a hook under an interception point (see [`HookPoint`]).
    pub fn hook(&mut self, point: HookPoint, hook: Arc<dyn EventHook>) {
        self.reg
            .tables_mut()
            .hooks
            .entry(point)
            .or_default()
            .push(HookReg {
                module: self.module.clone(),
                hook,
            });
    }

    /// Subscribe a synchronous observer to one world-event kind.
    pub fn subscribe(&mut self, kind: WorldEventKind, observer: Arc<dyn WorldObserver>) {
        self.reg.tables_mut().observers.push(ObserverReg {
            kind,
            module: self.module.clone(),
            observer,
        });
    }

    /// Declare a typed state slot, installed on the world at engine
    /// construction and reachable via [`Ctx::state`] / [`Ctx::state_mut`].
    /// Two **different** modules declaring the same slot type is a boot
    /// error (checked by `Engine::with_modules`); a module re-declaring its
    /// own slot keeps the last value.
    pub fn state<T: Any + Send + Clone>(&mut self, init: T) {
        self.reg.tables_mut().state_inits.push(StateInit {
            type_id: TypeId::of::<T>(),
            module: self.module.clone(),
            value: Box::new(init),
            clone_fn: clone_state::<T>,
        });
    }

    /// Reserve a panel feed id and get its emitting handle.
    pub fn panel_feed(&mut self, name: &str) -> Result<PanelFeedHandle, ModError> {
        if let Some((n, owner)) = self
            .reg
            .panel_feeds()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "panel feed",
                name: n.clone(),
                owner: owner.clone(),
            });
        }
        let feed = SmolStr::new(name);
        self.reg
            .tables_mut()
            .panel_feeds
            .push((feed.clone(), self.module.clone()));
        Ok(PanelFeedHandle { feed })
    }

    /// Register a named read-only query handler (web `Req` hydration —
    /// registry only until M4).
    pub fn query_handler(
        &mut self,
        name: &str,
        handler: Arc<dyn QueryHandler>,
    ) -> Result<(), ModError> {
        let key = SmolStr::new(name);
        if let Some((owner, _)) = self.reg.query_handler(&key) {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "query handler",
                name: key,
                owner: owner.clone(),
            });
        }
        self.reg
            .tables_mut()
            .query_handlers
            .insert(key, (self.module.clone(), handler));
        Ok(())
    }

    /// Declare a job queue (registry only — the jobs module consumes it).
    pub fn job_queue(&mut self, spec: JobQueueSpec) -> Result<(), ModError> {
        if let Some(q) = self
            .reg
            .job_queues()
            .iter()
            .find(|q| q.name.eq_ignore_ascii_case(&spec.name))
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "job queue",
                name: q.name.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg.tables_mut().job_queues.push(spec);
        Ok(())
    }

    /// Reserve a Rhai API namespace (`mush::<name>` — name registry only;
    /// Rhai lands M5).
    pub fn script_api(&mut self, name: &str) -> Result<(), ModError> {
        if let Some((n, owner)) = self
            .reg
            .script_apis()
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "script api",
                name: n.clone(),
                owner: owner.clone(),
            });
        }
        self.reg
            .tables_mut()
            .script_apis
            .push((SmolStr::new(name), self.module.clone()));
        Ok(())
    }

    /// Collect namespaced migrations for the driver to run against the
    /// store at boot (BGSAVE-before-migrate is mandatory there).
    ///
    /// The first module to register migrations for a namespace **owns** it:
    /// a second module extending the same namespace is a conflict (two
    /// modules' merged lists would fail the strictly-ascending version
    /// check at boot with a confusing error — refuse it here instead). The
    /// owning module may call this again to append more steps.
    pub fn migrations(&mut self, ns: &str, ms: Vec<Migration>) -> Result<(), ModError> {
        let key = SmolStr::new(ns);
        let tables = self.reg.tables_mut();
        if let Some(owner) = tables.migration_owners.get(&key)
            && owner != &self.module
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "migration namespace",
                name: key,
                owner: owner.clone(),
            });
        }
        tables
            .migration_owners
            .insert(key.clone(), self.module.clone());
        tables.migrations.entry(key).or_default().extend(ms);
        Ok(())
    }

    /// Register the game's single weather provider (the reserved
    /// `WeatherQuery` seam — see [`WeatherQuery`]). Exactly one module may
    /// provide weather; a second registration is a conflict.
    pub fn weather_query(&mut self, provider: Arc<dyn WeatherQuery>) -> Result<(), ModError> {
        if let Some((owner, _)) = &self.reg.tables_mut().weather {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "weather provider",
                name: SmolStr::new_static("weather"),
                owner: owner.clone(),
            });
        }
        self.reg.tables_mut().weather = Some((self.module.clone(), provider));
        Ok(())
    }

    /// Register this module's [`AsyncCompletion`] handler — the re-entry
    /// point for its deferred async work
    /// ([`Effect::ModuleAsync`](crate::engine::Effect::ModuleAsync)). A module
    /// registers at most one; a second registration is a conflict.
    pub fn async_completion(&mut self, handler: Arc<dyn AsyncCompletion>) -> Result<(), ModError> {
        if self
            .reg
            .tables_mut()
            .async_completions
            .contains_key(&self.module)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "async completion",
                name: self.module.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg
            .tables_mut()
            .async_completions
            .insert(self.module.clone(), handler);
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Firing helpers (used by dispatch, movement, engine)
// ----------------------------------------------------------------------

/// Consult every `MoveAttempt` hook; the first veto wins. `exit` is `None`
/// for `@teleport`.
pub fn check_move_attempt(
    reg: &Registry,
    w: &mut World,
    actor: ObjId,
    exit: Option<ObjId>,
    dest: ObjId,
) -> Result<(), RichText> {
    for h in reg.hooks(HookPoint::MoveAttempt) {
        h.hook.move_attempt(w, actor, exit, dest)?;
    }
    Ok(())
}

/// The shared perception decision over an explicit `Perceive`-hook slice:
/// `observer` perceives `subject` unless some hook returns
/// [`Perceive::Conceal`] (first-conceal-wins). An empty slice is a free pass
/// (returns `true` without touching the world) — the fast path that keeps a
/// hookless game byte-for-byte identical to today.
///
/// This is the ONE place the projection rule lives. [`can_perceive`] wraps it
/// for callers holding a live [`Registry`]; the off-loop script runtime
/// snapshots the same slice and calls this directly, so the two consumers can
/// never drift.
#[must_use]
pub fn perceives_via(hooks: &[HookReg], w: &World, observer: ObjId, subject: ObjId) -> bool {
    for h in hooks {
        if h.hook.perceives(w, observer, subject) == Perceive::Conceal {
            return false;
        }
    }
    true
}

/// Whether `observer` may perceive `subject`, consulting every registered
/// `Perceive` hook (first-conceal-wins; see [`perceives_via`]). Read-only
/// `&World` so both the `look`/`examine` render path and the live
/// `Audience::Room` fan-out can call it. With no `Perceive` hook registered
/// this is a free pass, preserving today's behavior exactly.
#[must_use]
pub fn can_perceive(reg: &Registry, w: &World, observer: ObjId, subject: ObjId) -> bool {
    perceives_via(reg.hooks(HookPoint::Perceive), w, observer, subject)
}

/// Whether `subject` is under an active concealment effect according to ANY
/// registered `Perceive`-point hook (first-true-wins). The ruleset-agnostic
/// **notability** query — distinct from [`can_perceive`], which is the
/// per-observer projection: this asks "is this subject hidden at all?", not
/// "can THIS watcher see them?". With no hook registered (or none claiming
/// concealment) it is `false`, so it is additive and free — behavior identical
/// to a game with no concealment ruleset.
#[must_use]
pub fn is_concealed(reg: &Registry, w: &World, subject: ObjId) -> bool {
    reg.hooks(HookPoint::Perceive)
        .iter()
        .any(|h| h.hook.is_concealed(w, subject))
}

/// Collect every `LookRendered` hook's extra lines for a room render.
#[must_use]
pub fn look_rendered_lines(reg: &Registry, w: &World, looker: ObjId, room: ObjId) -> Vec<RichText> {
    let mut lines = Vec::new();
    for h in reg.hooks(HookPoint::LookRendered) {
        lines.extend(h.hook.look_rendered(w, looker, room));
    }
    lines
}

/// Invoke every observer subscribed to `ev`'s kind, post-mutation, each
/// with a [`Ctx`] carrying its module identity.
pub fn emit_world_event(reg: &Registry, w: &mut World, out: &mut Outbox, ev: &WorldEvent) {
    let observers = reg.observers_matching(ev);
    let actor = ev.subject();
    for (module, obs) in observers {
        let mut ctx = Ctx {
            w: &mut *w,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(module),
        };
        obs.on_event(&mut ctx, ev);
    }
}

/// Route a [`WorldCommand::ModuleAsyncDone`](crate::engine::WorldCommand::ModuleAsyncDone)
/// to `module`'s registered [`AsyncCompletion`] handler, in the world task,
/// with a full [`Ctx`] carrying the module identity. A missing handler is
/// logged and dropped (the completion arrived for a module that registered
/// none — a programming error, never a crash).
pub fn apply_async_completion(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    module: SmolStr,
    actor: ObjId,
    correlation: u64,
    payload: &str,
) {
    let Some(handler) = reg.async_completion(&module) else {
        tracing::warn!(%module, correlation, "async completion for unregistered module dropped");
        return;
    };
    let mut ctx = Ctx {
        w,
        actor,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
    };
    handler.on_complete(&mut ctx, correlation, payload);
}

/// Fire the `Connected` / `Disconnected` hooks for an actor.
pub fn fire_conn_hook(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    actor: ObjId,
    connected: bool,
) {
    let point = if connected {
        HookPoint::Connected
    } else {
        HookPoint::Disconnected
    };
    let hooks: Vec<HookReg> = reg.hooks(point).to_vec();
    for h in hooks {
        let mut ctx = Ctx {
            w: &mut *w,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(h.module.clone()),
        };
        if connected {
            h.hook.connected(&mut ctx, actor);
        } else {
            h.hook.disconnected(&mut ctx, actor);
        }
    }
}

/// Fire the `SceneStarted` / `SceneEnded` hooks for a scene (the scene is
/// still queryable during `SceneEnded`).
pub fn fire_scene_hook(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    scene: SceneId,
    started: bool,
) {
    let point = if started {
        HookPoint::SceneStarted
    } else {
        HookPoint::SceneEnded
    };
    let hooks: Vec<HookReg> = reg.hooks(point).to_vec();
    for h in hooks {
        let mut ctx = Ctx {
            w: &mut *w,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(h.module.clone()),
        };
        if started {
            h.hook.scene_started(&mut ctx, scene);
        } else {
            h.hook.scene_ended(&mut ctx, scene);
        }
    }
}

// ----------------------------------------------------------------------
// Ctx extension: the module-facing surface
// ----------------------------------------------------------------------

impl Ctx<'_> {
    /// Queue a module persistence op onto the command's deferred
    /// side-channel. The world task drains these into the current
    /// `WriteBatch` — the write commits atomically with this command's
    /// object mutations.
    ///
    /// **A handler that returns `Err` discards everything it deferred**
    /// (persistence ops, timers, effects): dispatch truncates the deferred
    /// queue back to its pre-handler length, so a validation failure after
    /// `persist` calls never half-commits. World mutations made before the
    /// error remain best-effort — validate-then-apply is still the rule.
    ///
    /// **Trust model**: `op.ns` is not verified against `ctx.module`.
    /// Compiled-in modules are trusted code; writing into another module's
    /// namespace is *sanctioned delegation* (e.g. producing modules write
    /// `jobs`-namespace docs through the jobs module's exported helpers)
    /// and must only ever be done through the owning module's public API.
    /// Any future enforcement will formalize that delegation rather than
    /// check `ns == module`.
    pub fn persist(&mut self, op: ModuleOp) {
        self.out.defer(Deferred::Persist(op));
    }

    /// Gate this step's output on the store's durable acknowledgement of
    /// its write batch (PLAN: "durability-critical ops — account creation,
    /// mail send, XP approval — gate their confirmation output on the
    /// ack"). The world task flushes the step's batch with `durable: true`
    /// and holds **all** of the step's queued output until the store acks;
    /// if the write ultimately fails, the acting player is warned. Use it
    /// from the command that emits a "sent"/"approved" confirmation whose
    /// loss would be a lie after a crash.
    pub fn gate_on_durable_ack(&mut self) {
        self.out.defer(Deferred::DurableGate { actor: self.actor });
    }

    /// Read this world's state slot for `T` (declared via
    /// [`Registrar::state`]).
    #[must_use]
    pub fn state<T: Any + Send>(&self) -> Option<&T> {
        self.w.module_state::<T>()
    }

    /// Mutably access this world's state slot for `T`.
    pub fn state_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        self.w.module_state_mut::<T>()
    }

    /// Emit a world event to subscribed observers (post-mutation). Command
    /// handlers that mutate the world through paths not covered by the
    /// core helpers call this themselves.
    pub fn emit_world_event(&mut self, ev: &WorldEvent) {
        emit_world_event(self.reg, self.w, self.out, ev);
    }

    // ------------------------------------------------------------------
    // IC game clock
    // ------------------------------------------------------------------
    //
    // Thin delegators onto the [`World`] read model so hooks, ticks, and
    // (later) scripts read IC time without reaching for `ctx.w.now()` (which
    // is OOC wall time). The recommended way for a module to notice a
    // day/season rollover is to read [`Ctx::season`] / [`Ctx::ic_day_of_year`]
    // in its `Coarse` tick and compare against the value it saw last — no new
    // world event is needed, and the clock is a pure function of wall time so
    // the check is cheap.

    /// IC unix-style timestamp right now (the game clock applied to wall
    /// time).
    #[must_use]
    pub fn game_now_unix(&self) -> i64 {
        self.w.game_now_unix()
    }

    /// The IC season right now.
    #[must_use]
    pub fn season(&self) -> Season {
        self.w.season()
    }

    /// The IC time-of-day bucket right now.
    #[must_use]
    pub fn time_of_day(&self) -> TimeOfDay {
        self.w.time_of_day()
    }

    /// IC hour of day right now, `0..24`.
    #[must_use]
    pub fn ic_hour(&self) -> u32 {
        self.w.ic_hour()
    }

    /// IC minute of the hour right now, `0..60`.
    #[must_use]
    pub fn ic_minute(&self) -> u32 {
        self.w.ic_minute()
    }

    /// IC day of the current month right now, `1..=` its length.
    #[must_use]
    pub fn ic_day(&self) -> u32 {
        self.w.ic_day()
    }

    /// IC day of the current year right now, `0..365` (rollover signal for
    /// ticking modules).
    #[must_use]
    pub fn ic_day_of_year(&self) -> u32 {
        self.w.ic_day_of_year()
    }

    /// IC month right now, `1..=12`.
    #[must_use]
    pub fn ic_month(&self) -> u32 {
        self.w.ic_month()
    }

    /// IC year right now.
    #[must_use]
    pub fn ic_year(&self) -> i64 {
        self.w.ic_year()
    }

    /// The human-facing IC datetime string right now.
    #[must_use]
    pub fn ic_datetime(&self) -> String {
        self.w.ic_datetime()
    }

    // ------------------------------------------------------------------
    // Scenes
    // ------------------------------------------------------------------

    /// Start a scene of `kind` with `members`, firing `SceneStarted`
    /// hooks. The scene records **this module** as its persistence
    /// namespace (falling back to `kind` for identity-less callers), so
    /// every later scene write — from any caller — lands in the creating
    /// module's namespace.
    pub fn start_scene(&mut self, kind: &str, members: BTreeSet<ObjId>) -> SceneId {
        let ns = self.module.clone().unwrap_or_else(|| SmolStr::new(kind));
        let id = self.w.scene_create(kind, &ns, members);
        fire_scene_hook(self.reg, self.w, self.out, id, true);
        id
    }

    /// End a scene: fires `SceneEnded` hooks (scene still queryable), then
    /// removes it, its per-object ruleset state, and its map blob, queuing
    /// `Del` persistence ops for both (in the scene's **owning** namespace,
    /// regardless of who calls this). Scene ticks stop on their next due
    /// time. Returns whether the scene existed.
    pub fn end_scene(&mut self, id: SceneId) -> bool {
        if self.w.scene(id).is_none() {
            return false;
        }
        fire_scene_hook(self.reg, self.w, self.out, id, false);
        let Some(scene) = self.w.scene_remove(id) else {
            return false;
        };
        let ns = scene.ns.clone();
        self.persist(ModuleOp {
            ns: ns.clone(),
            op: ModuleWrite::Del {
                key: scene.ruleset_state_key.clone(),
            },
        });
        self.persist(ModuleOp {
            ns,
            op: ModuleWrite::Del {
                key: SmolStr::new(map_blob_key(id)),
            },
        });
        true
    }

    /// Start this module's registered [`TickCadence::Scene`] tick `tag`
    /// firing every `every` for `scene` (first fire after one period).
    /// Restarting an already-running tick re-arms it with the new period.
    /// The tick stops when the scene ends or via [`Ctx::scene_tick_stop`].
    pub fn scene_tick_start(
        &mut self,
        scene: SceneId,
        tag: &str,
        every: Duration,
    ) -> Result<(), CmdError> {
        let Some(module) = self.module.clone() else {
            return Err(CmdError::msg("Scene ticks require a module identity."));
        };
        if self.w.scene(scene).is_none() {
            return Err(CmdError::msg("No such scene."));
        }
        match self.reg.tick_reg(&module, tag).map(|t| t.cadence) {
            Some(TickCadence::Scene) => {}
            _ => {
                return Err(CmdError::msg(format!(
                    "No scene tick {tag:?} registered by module {module:?}."
                )));
            }
        }
        self.out.defer(Deferred::SceneTickStart {
            module,
            tag: SmolStr::new(tag),
            scene,
            every_ms: duration_ms(every),
        });
        Ok(())
    }

    /// Stop a running scene tick (no-op if it is not running).
    pub fn scene_tick_stop(&mut self, scene: SceneId, tag: &str) {
        let Some(module) = self.module.clone() else {
            return;
        };
        self.out.defer(Deferred::SceneTickStop {
            module,
            tag: SmolStr::new(tag),
            scene,
        });
    }

    /// Read one `(scene, obj)` ruleset-state value.
    #[must_use]
    pub fn scene_state(&self, scene: SceneId, obj: ObjId) -> Option<&serde_json::Value> {
        self.w.scene_state(scene, obj)
    }

    /// Set one `(scene, obj)` ruleset-state value and persist a snapshot
    /// of the whole scene's state under the scene's `ruleset_state_key`
    /// (naive-but-real: one `JsonSet` per change).
    pub fn scene_state_set(
        &mut self,
        scene: SceneId,
        obj: ObjId,
        value: serde_json::Value,
    ) -> Result<(), CmdError> {
        let (ns, key) = self.scene_persist_target(scene)?;
        self.w.scene_state_set(scene, obj, value);
        let snapshot = self.w.scene_state_snapshot(scene);
        self.persist(ModuleOp {
            ns,
            op: ModuleWrite::JsonSet {
                key,
                value: snapshot,
            },
        });
        Ok(())
    }

    /// Read a scene's map blob.
    #[must_use]
    pub fn map_blob(&self, scene: SceneId) -> Option<&serde_json::Value> {
        self.w.scene_map(scene)
    }

    /// Set a scene's map blob (core never interprets it) and persist it.
    pub fn set_map_blob(
        &mut self,
        scene: SceneId,
        value: serde_json::Value,
    ) -> Result<(), CmdError> {
        let (ns, _) = self.scene_persist_target(scene)?;
        self.w.scene_map_set(scene, value.clone());
        self.persist(ModuleOp {
            ns,
            op: ModuleWrite::JsonSet {
                key: SmolStr::new(map_blob_key(scene)),
                value,
            },
        });
        Ok(())
    }

    /// The `(namespace, state key)` a scene's module data persists under:
    /// always the scene's **owning** namespace (recorded at creation), so
    /// two modules touching one scene never scatter its documents.
    fn scene_persist_target(&self, scene: SceneId) -> Result<(SmolStr, SmolStr), CmdError> {
        let sc = self
            .w
            .scene(scene)
            .ok_or_else(|| CmdError::msg("No such scene."))?;
        Ok((sc.ns.clone(), sc.ruleset_state_key.clone()))
    }

    // ------------------------------------------------------------------
    // Input contexts
    // ------------------------------------------------------------------

    /// Push an input context onto the actor's stack and emit its prompt.
    /// Until it pops, the actor's raw lines feed the context instead of
    /// normal dispatch (`~q` always escapes). The context is tagged with
    /// **this handler's module identity**, which every fed line's `Ctx`
    /// carries — so a module's context keeps its attr-namespace and scene
    /// privileges.
    pub fn push_input_context(&mut self, cx: Box<dyn InputContext>) {
        let spec = cx.prompt(self.w);
        self.w
            .push_input_context(self.actor, self.module.clone(), cx);
        self.out.to_actor(self.actor, OutputEvent::Prompt(spec));
    }

    /// Pop the actor's top input context (if any) and restore the previous
    /// prompt (the next context's, or the normal prompt). Returns whether
    /// a context was popped.
    pub fn pop_input_context(&mut self) -> bool {
        if self.w.pop_input_context(self.actor).is_none() {
            return false;
        }
        let spec = restored_prompt(self.w, self.actor);
        self.out.to_actor(self.actor, OutputEvent::Prompt(spec));
        true
    }
}

/// The prompt to show after a pop: the (new) top context's, or the normal
/// input mode.
#[must_use]
pub(crate) fn restored_prompt(w: &World, actor: ObjId) -> PromptSpec {
    w.top_input_prompt(actor).unwrap_or(PromptSpec {
        mode: mush_proto::InputMode::Normal,
        text: None,
    })
}

/// The persistence key a scene's map blob lives under.
#[must_use]
pub fn map_blob_key(scene: SceneId) -> String {
    format!("scene:{}:map", scene.0)
}

/// A `Duration` as clamped milliseconds (>= 1).
#[must_use]
pub(crate) fn duration_ms(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX).max(1)
}

#[cfg(test)]
mod concealment_tests {
    use super::*;
    use crate::obj::ObjId;
    use crate::world::World;

    /// With no `Perceive` hook registered, the notability query defaults to
    /// `false` — additive and free (a game with no concealment ruleset).
    #[test]
    fn is_concealed_defaults_false_with_no_hook() {
        let reg = Registry::new();
        let w = World::new_minimal();
        assert!(!is_concealed(&reg, &w, ObjId(1)));
        // And `can_perceive` remains a free pass on the same empty registry.
        assert!(can_perceive(&reg, &w, ObjId(1), ObjId(2)));
    }

    /// A registered hook whose `is_concealed` returns true makes the firing
    /// helper report the subject concealed (first-true-wins).
    #[test]
    fn is_concealed_true_when_a_hook_claims_it() {
        struct AlwaysConcealed;
        impl EventHook for AlwaysConcealed {
            fn is_concealed(&self, _w: &World, _subject: ObjId) -> bool {
                true
            }
        }
        let mut reg = Registry::new();
        Registrar::new(&mut reg, "test").hook(HookPoint::Perceive, Arc::new(AlwaysConcealed));
        let w = World::new_minimal();
        assert!(is_concealed(&reg, &w, ObjId(1)));
    }
}
