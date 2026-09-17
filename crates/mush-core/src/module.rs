//! The merged module API (treaty ruling 4): [`GameModule`] + [`Registrar`].
//!
//! Modules are compiled-in crates handed to
//! [`Engine::compose_modules`](crate::engine::Engine::compose_modules) as
//! `Box<dyn GameModule>` **before** persisted world load. Each module's
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
//! - scenes (the tactical seam) via [`Ctx::start_scene`] /
//!   [`Ctx::end_scene`] / scene ticks / the per-scene state store and map
//!   blob.
//!
//! Comsys, mail, and jobs are built on this API first (M3) as its proving
//! ground; the tactics stub keeps the scene/tick/veto seams green forever.

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
    /// J4 (doc 12 §5.6): the module was handed a **narrowed** namespace (a
    /// boot manifest let the loader skip cold ledger chunks) and cannot boot
    /// from it — the checkpoint the manifest stood for proved invalid or
    /// absent — so the driver must fetch the namespace whole, manifest
    /// ignored, and call [`GameModule::on_world_loaded`] once more. Returned
    /// only for a narrowed fetch; from a whole one it is a plain failure.
    #[error("module {module:?} needs its whole namespace: {reason}")]
    RetryFullFetch {
        /// The module asking.
        module: SmolStr,
        /// Why the narrowed data would not do.
        reason: String,
    },
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

/// Maximum UTF-8 bytes in a module persistence namespace. Shared by the
/// producer, store, and quarantine sink so an accepted authoritative operation
/// can always be represented by its recovery envelope.
pub const MAX_MODULE_NAMESPACE_BYTES: usize = 256;

/// Maximum UTF-8 bytes in a key within a module persistence namespace.
pub const MAX_MODULE_KEY_BYTES: usize = 4 * 1024;

/// The most keys one [`Ctx::read_store`] request may name (H1). A page of
/// ledger chunks is the intended shape — a handful of `log:{id}:{n}`
/// documents — never a namespace listing.
pub const MAX_STORE_READ_KEYS: usize = 16;

/// The most bytes one store read may return, summed over the encoded
/// documents (H1). Past this the driving loop re-enters with
/// [`StoreReadResult::Oversized`] and **no** documents, so a module cannot
/// pull an unbounded slice of history into the world loop. Sized for
/// `MAX_STORE_READ_KEYS` ledger chunks at their 16 KiB target with headroom,
/// well under the 8 MiB single-chunk safety ceiling.
pub const MAX_STORE_READ_BYTES: usize = 512 * 1024;

/// How long the driving loop lets one store read run before it re-enters
/// with [`StoreReadResult::Failed`] (H1). A read is a view, so it is never
/// retried; the requester may simply ask again.
pub const STORE_READ_TIMEOUT_MS: u64 = 5_000;

/// How old (world seconds) a module's pending-read bookkeeping may be before
/// a new request from the same actor supersedes it (H1 verify fold, B1).
/// A read that launched answers within [`STORE_READ_TIMEOUT_MS`]; one that
/// never launched — the effect refused while the world was booting or
/// recovering, or the completion dropped for the same reason — answers never,
/// and without this window that actor could not read again until a restart.
/// A late completion for the superseded correlation is then a stale drop.
pub const STORE_READ_STALE_SECS: i64 = 30;

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
    /// Every `mod:{ns}:{key}` JSON document that is **not** a ledger chunk:
    /// `(key, value)` pairs.
    pub kv: Vec<(SmolStr, serde_json::Value)>,
    /// Every ledger chunk document (`log:{id}:{chunk}`) as raw JSON text
    /// (J1, doc 12 F9): the store hands these over unparsed, so a journal's
    /// boot footprint is its byte size, and the one inspection
    /// ([`crate::inspect_ledger`]) every consumer shares decodes each chunk
    /// exactly once. A `log:` key that arrives in `kv` instead (a test rig, an
    /// older loader) is moved here by [`ModuleBootData::normalize`] before the
    /// module sees it.
    pub ledger: Vec<crate::LedgerDoc>,
    /// Named stream tails, oldest-first: stream key → `(entry id, value)`.
    pub streams: HashMap<SmolStr, Vec<(String, serde_json::Value)>>,
    /// How long the store fetch of this namespace took, in milliseconds
    /// (doc 12 D-12-3: the part of a boot the replay timer never saw). Zero
    /// when nobody measured it (a test rig).
    pub fetch_ms: u64,
    /// J4 (doc 12 §5.6): what the loader **deliberately did not fetch** —
    /// per ledger log id, the chunk index below which no chunk document was
    /// read, as the namespace's [`BootManifest`] asked. The min-present
    /// marker the ledger inspection honours ([`crate::inspect_ledger_from`]);
    /// empty for a whole fetch. A module that finds it cannot boot from a
    /// narrowed namespace returns [`ModError::RetryFullFetch`].
    pub omitted: std::collections::BTreeMap<SmolStr, u64>,
}

/// The key of a namespace's boot manifest (J4, doc 12 §5.6): a document the
/// **module** writes and the **loader** reads before listing the namespace,
/// so a checkpointed boot never fetches the cold ledger chunks it will not
/// decode. Loader-owned protocol: it is stripped from the boot data's `kv`
/// and never reaches the module as a document.
pub const BOOT_MANIFEST_KEY: &str = "boot-manifest";

/// The current [`BootManifest::format`].
pub const BOOT_MANIFEST_FORMAT: u32 = 1;

/// What a namespace's boot may skip (J4, doc 12 §5.6), written by the module
/// under [`BOOT_MANIFEST_KEY`]. The loader honours a manifest of a format it
/// knows and ignores any other (a whole fetch, said at `warn`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootManifest {
    /// [`BOOT_MANIFEST_FORMAT`] when written by this code.
    pub format: u32,
    /// Per ledger log id, the chunk index below which the loader need not
    /// fetch: the module has a checkpoint covering those chunks and the RAM
    /// tail it keeps starts at or above the named chunk. A zero entry asks
    /// for nothing.
    #[serde(default)]
    pub skip_log_chunks_below: std::collections::BTreeMap<SmolStr, u64>,
    /// Key prefixes the loader need not fetch at all (doc 13 W1): documents
    /// the module reads through the store-read seam on demand and never at
    /// boot — the economy's `idem:` key documents. Applies to every key,
    /// ledger or not; an empty prefix is ignored.
    #[serde(default)]
    pub skip_key_prefixes: Vec<SmolStr>,
}

impl BootManifest {
    /// A manifest that skips nothing, at the current format.
    #[must_use]
    pub fn empty() -> Self {
        BootManifest {
            format: BOOT_MANIFEST_FORMAT,
            skip_log_chunks_below: std::collections::BTreeMap::new(),
            skip_key_prefixes: Vec::new(),
        }
    }

    /// The key prefixes the loader skips: every non-empty one of a manifest
    /// at a format this code knows; nothing otherwise.
    #[must_use]
    pub fn prefixes(&self) -> Vec<SmolStr> {
        if self.format != BOOT_MANIFEST_FORMAT {
            return Vec::new();
        }
        self.skip_key_prefixes
            .iter()
            .filter(|p| !p.is_empty())
            .cloned()
            .collect()
    }

    /// The floors the loader applies: every non-zero entry of a manifest at a
    /// format this code knows; nothing otherwise.
    #[must_use]
    pub fn floors(&self) -> std::collections::BTreeMap<SmolStr, u64> {
        if self.format != BOOT_MANIFEST_FORMAT {
            return std::collections::BTreeMap::new();
        }
        self.skip_log_chunks_below
            .iter()
            .filter(|(_, below)| **below > 0)
            .map(|(log, below)| (log.clone(), *below))
            .collect()
    }
}

impl ModuleBootData {
    /// Move any ledger chunk documents that arrived in `kv` into `ledger` as
    /// text, so a module boots from one shape whichever way its data came.
    /// Idempotent; a no-op when `kv` holds no `log:` key.
    pub fn normalize(&mut self) {
        // Every `log:`-prefixed key, well-formed or not: a malformed chunk key
        // is the inspection's `MalformedKey` breach to report, not a stray doc.
        let is_ledger = |key: &str| key.starts_with(crate::ledger::LOG_PREFIX);
        if !self.kv.iter().any(|(key, _)| is_ledger(key)) {
            return;
        }
        let kv = std::mem::take(&mut self.kv);
        for (key, value) in kv {
            if is_ledger(&key) {
                let text =
                    serde_json::to_string(&value).expect("a serde_json::Value always serializes");
                self.ledger.push((key, text));
            } else {
                self.kv.push((key, value));
            }
        }
    }
}

// ----------------------------------------------------------------------
// Scenes (the tactics seam)
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
/// WoD uses scenes lightly (roll logs, beat awards); tactics binds a tactical window
/// and a fast ticker to one.
///
/// **Boot contract**: scenes are transient world state and scene ids
/// restart at 0 every boot, but a crash without [`Ctx::end_scene`] leaves
/// the scene's persisted `scene:{id}:*` documents behind. A module that
/// persists scene state MUST purge stale `scene:*` keys from its namespace
/// in [`GameModule::on_world_loaded`] (queue `Del` ops for them), or the
/// next boot's scene #0 interleaves with the orphan's keys. The tactics stub
/// demonstrates the pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene {
    /// The scene's id.
    pub id: SceneId,
    /// Module-defined kind, e.g. `"tactics.battle"`.
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
    /// Veto-capable: consulted by `get`, `drop`, and `give` **before** the
    /// carried object moves. Deliberately separate from `MoveAttempt`: those
    /// are actor-enters-destination moves that emit [`WorldEvent::Moved`];
    /// carry moves emit no event, and a hook that vetoes *destinations* (a
    /// contested room) must not fire when someone drops a wrench there.
    /// `Err(reason)` blocks the carry and shows the reason to the mover.
    CarryAttempt,
    /// Append lines to a rendered room view (weather ambience, scene
    /// banners). Fires wherever a room render is shown: `look`, movement
    /// arrival, `@teleport`.
    LookRendered,
    /// Contribute structured ambience to a rendered room view — the
    /// `RoomView.ambient` map a client's room band or status line reads
    /// (interface plan I1, D-I-4). Fires wherever `LookRendered` does; each
    /// hook returns `(key, value)` pairs, first writer wins on a key, and the
    /// whole map is bounded ([`MAX_AMBIENT_KEYS`], [`MAX_AMBIENT_BYTES`]).
    RoomAmbient,
    /// Read-only per-observer perception gate (the M11 projection seam):
    /// consulted wherever the output layer decides whether one observer may
    /// perceive one subject — the `look`/`examine` contents filters and the
    /// live `Audience::Room` fan-out. First `Conceal` wins; with no hook
    /// registered the gate is a free pass (behavior byte-for-byte identical
    /// to a game with no concealment ruleset). See [`can_perceive`].
    Perceive,
    /// An actor's session became live (first connection attached).
    ///
    /// **16b cut K.2 (K4).** A *revival* inside the session layer's linkdead
    /// grace fires this same point's registrations, but on
    /// [`EventHook::revived`] rather than [`EventHook::connected`] — see
    /// [`ConnTransition`] for why the flag selects a second method instead of
    /// widening `connected`.
    Connected,
    /// An actor's last connection dropped **uncleanly** and the session layer
    /// has not given it up yet: the character stays in the room, still
    /// `connected` for `WHO`, until the grace expires (doc 07 D1).
    ///
    /// **16b cut K.2 (K4).** The kernel announced this to the room and told no
    /// module about it, so a subsystem holding live authority on an actor's
    /// behalf — the arena's cockpit seat — could not tell a dropped line from a
    /// pilot who had gone quiet. A hook here is *not* a disconnect: the seat is
    /// kept and the tank fires nothing (doc 16 §3.8 "Disconnects"), and
    /// [`Disconnected`](HookPoint::Disconnected) is still what fires when the
    /// grace runs out.
    Linkdead,
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

/// Immutable, owner-scoped truth available to veto and projection hooks.
///
/// The context exposes no mutation, output, scheduling, or clock surface.
/// Its typed reader is stamped from the hook's registering module, so an
/// opaque family handle cannot be used across owners.
pub struct HookReadContext<'a> {
    world: &'a World,
    typed_state: crate::action_state::ModuleActionState<'a>,
    registry: Option<&'a Registry>,
}

impl<'a> HookReadContext<'a> {
    /// Construct a compatibility context for an off-loop hook call.
    ///
    /// Typed state is deliberately unavailable in this context — the lens is
    /// [`DetachedTypedState::CompatibilityCtx`](crate::DetachedTypedState::CompatibilityCtx).
    /// Engine-owned dispatch paths use the private constructor below to attach
    /// the live authoritative snapshot.
    #[must_use]
    pub fn without_typed_state(world: &'a World, module: &str) -> Self {
        Self::for_module(
            world,
            crate::action_state::TypedStateLens::Detached(
                crate::action_state::DetachedTypedState::CompatibilityCtx,
            ),
            module,
        )
    }

    pub(crate) fn for_module(
        world: &'a World,
        lens: crate::action_state::TypedStateLens<'a>,
        module: &str,
    ) -> Self {
        Self {
            world,
            typed_state: crate::action_state::ModuleActionState::for_module(lens, Some(module)),
            registry: None,
        }
    }

    /// Attach the live registry (engine query dispatch only), so a read-only
    /// query can project registry-owned tables such as the command list.
    pub(crate) fn with_registry(mut self, registry: &'a Registry) -> Self {
        self.registry = Some(registry);
        self
    }

    #[must_use]
    pub const fn world(&self) -> &'a World {
        self.world
    }

    /// The live registry when the engine supplied one (query dispatch);
    /// `None` for off-loop and hook contexts.
    #[must_use]
    pub const fn registry(&self) -> Option<&'a Registry> {
        self.registry
    }

    #[must_use]
    pub const fn typed_state_available(&self) -> bool {
        self.typed_state.is_available()
    }

    pub fn action_state<K, V>(
        &self,
        handle: &crate::action_state::StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&'a V>, crate::action_state::ModuleActionStateReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.typed_state.get(handle, key)
    }

    pub fn scan_action_state<K, V>(
        &self,
        handle: &crate::action_state::StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<
        crate::action_state::StateFamilyPage<'a, V>,
        crate::action_state::ModuleActionStateReadError,
    >
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.typed_state.scan_family(handle, after, limit)
    }
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
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        Ok(())
    }

    /// Veto a carry attempt: `mover` is picking up, dropping, or giving `obj`
    /// so that it lands in `dest` (the mover, a room, or a recipient). Return
    /// `Err(reason)` to block; the reason is shown to the mover.
    fn carry_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        mover: ObjId,
        obj: ObjId,
        dest: ObjId,
    ) -> Result<(), RichText> {
        Ok(())
    }

    /// Extra lines appended after a room render for `looker`.
    fn look_rendered(
        &self,
        ctx: &HookReadContext<'_>,
        looker: ObjId,
        room: ObjId,
    ) -> Vec<RichText> {
        Vec::new()
    }

    /// Structured ambience for a room render: `(key, value)` pairs merged
    /// into `RoomView.ambient` under [`HookPoint::RoomAmbient`]'s rules
    /// (first writer per key; bounded). A module contributes its own shape
    /// under its own key (`"weather"`, `"scene"`) and never rewrites another's.
    fn room_ambient(
        &self,
        ctx: &HookReadContext<'_>,
        looker: ObjId,
        room: ObjId,
    ) -> Vec<(SmolStr, serde_json::Value)> {
        Vec::new()
    }

    /// Whether `observer` should be prevented from perceiving `subject`
    /// (the M11 per-observer projection gate). Read-only by contract — the
    /// same `&World` is shared by the look path and the live fan-out, so the
    /// verdict must be a cheap lookup, never a mutation (no rolling, no cache
    /// writes; a ruleset rolls and caches in its mutable paths and only reads
    /// the cache here). Default [`Perceive::Abstain`] leaves the subject
    /// visible. See [`can_perceive`].
    fn perceives(&self, ctx: &HookReadContext<'_>, observer: ObjId, subject: ObjId) -> Perceive {
        Perceive::Abstain
    }

    /// Whether `subject` is under an *active* concealment effect (e.g. WoD
    /// Obfuscate) — the ruleset-agnostic **notability** signal, distinct from
    /// [`perceives`](EventHook::perceives) (which is the per-observer
    /// projection). Read-only by the same contract as `perceives`: a cheap
    /// cache lookup, never a mutation. Default `false` — with no concealment
    /// ruleset a subject is never concealed. See [`is_concealed`].
    fn is_concealed(&self, ctx: &HookReadContext<'_>, subject: ObjId) -> bool {
        false
    }

    /// An actor connected (session became live).
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {}

    /// An actor's session became live **again**, inside the session layer's
    /// linkdead grace (16b cut K.2, K4). Fires on
    /// [`HookPoint::Connected`]'s registrations, and defaults to nothing.
    ///
    /// The default is the whole of the compatibility argument: before this cut a
    /// revival fired no hook at all, so every module that implements
    /// [`connected`](EventHook::connected) and not this keeps its behaviour
    /// exactly — a reconnecting player is not sent their mail count, their
    /// channel notices or their weather line a second time. A module that holds
    /// authority across the gap (the arena's cockpit seat) overrides it.
    fn revived(&self, ctx: &mut Ctx<'_>, actor: ObjId) {}

    /// An actor's last connection dropped uncleanly and the grace is running
    /// (16b cut K.2, K4). The actor is still `connected` world-side; see
    /// [`HookPoint::Linkdead`].
    fn linkdead(&self, ctx: &mut Ctx<'_>, actor: ObjId) {}

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
    /// A mechanical action (a dice roll) resolved — the scene system records
    /// it into the active scene's event log.
    Rolled,
    /// A canon event was recorded — the general seam by which the world's
    /// consequences drive further simulation (faction clocks advancing from
    /// scene outcomes). Observers react to the carried kind/entities.
    CanonRecorded,
    /// H2b: a turn-based scene handed its turn to an NPC.
    SceneTurn,
    /// Interface plan I13: typed authoritative state (an action-state family)
    /// went live after an exact commit receipt — the seam the
    /// `<ruleset>.vitals` republish subscribes to beside `AttrSet`, since an
    /// overlay-only mutation (a Willpower spend, Condition-Monitor damage)
    /// never touches the sheet attribute.
    ActionStateChanged,
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
    /// `who` resolved a mechanical action (a dice roll) in `room` — the general
    /// seam a scene records into its event log. Rulesets emit it after a roll;
    /// the scene system logs it. `summary` is the human one-liner.
    Rolled {
        /// Who rolled.
        who: ObjId,
        /// The room it happened in.
        room: ObjId,
        /// A short human summary of the result.
        summary: String,
    },
    /// A canon event was just recorded (by the canon module). The general seam
    /// for consequence-driven simulation: a faction clock watching a canon
    /// `kind` advances when one is recorded involving its members.
    CanonRecorded {
        /// The canon event's durable id.
        canon_id: u64,
        /// Its module-defined kind (`"scene.concluded"`, …).
        kind: String,
        /// The entities it concerns.
        entities: Vec<ObjId>,
        /// Its free-form provenance (`"scene:5"`, `"faction:3"`, …) — the canon
        /// draft's own `source`, carried through so an observer can tell *which*
        /// front, operation or scene a recorded consequence came from.
        ///
        /// **The one `mush-core` change of E4** (08c §2, §6.2). The economy's
        /// attestation observer matches a contract condition on kind **and**
        /// source: 08e answer 20 requires progress to cite a specific front, so
        /// `scene.concluded` involving any faction member is deliberately not a
        /// match key, and `mush-canon` exposes no id→event query for an observer
        /// to look the source up with afterwards. It is a lifecycle-notification
        /// field, which doc 09 C6 keeps `WorldEvent` for.
        source: String,
    },
    /// H2b: `+timestop/next` handed the turn to `actor`, an `ObjKind::Npc`,
    /// in round `round` of scene `scene`. The NPC module turns this into a
    /// durable decision request; the scene does not auto-advance when the
    /// decision resolves.
    SceneTurn {
        /// The scene (its `SceneUid`).
        scene: u64,
        /// The NPC whose turn it is.
        actor: ObjId,
        /// The round the turn belongs to.
        round: u32,
    },
    /// Interface plan I13: the records of one action-state family changed and
    /// are live. Emitted by
    /// [`Engine::install_prepared_action_announcing`](crate::Engine::install_prepared_action_announcing)
    /// **after** the post-images are installed, once per touched
    /// `(owner, family)`, so an observer reads post-state. `keys` are the
    /// family's **encoded** record keys in key order — the owning module
    /// decodes its own codec (`VitalsSource::state_subject` in `mush-ruleset`);
    /// no value and no foreign decoding crosses this seam. Bounded by the
    /// transaction's own operation cap.
    ActionStateChanged {
        /// The module that owns the family.
        owner: crate::mechanics::OwnerId,
        /// The family whose records changed.
        family: crate::action_state::StateFamilyId,
        /// The encoded keys of the changed records, in key order.
        keys: Vec<SmolStr>,
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
            WorldEvent::Rolled { .. } => WorldEventKind::Rolled,
            WorldEvent::CanonRecorded { .. } => WorldEventKind::CanonRecorded,
            WorldEvent::SceneTurn { .. } => WorldEventKind::SceneTurn,
            WorldEvent::ActionStateChanged { .. } => WorldEventKind::ActionStateChanged,
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
            | WorldEvent::PowerUsed { who, .. }
            | WorldEvent::Rolled { who, .. } => *who,
            WorldEvent::SceneTurn { actor, .. } => *actor,
            WorldEvent::ObjCreated { creator, .. } => *creator,
            WorldEvent::ObjDestroyed { obj } | WorldEvent::AttrSet { obj, .. } => *obj,
            // A system consequence — no single actor; observers run as GOD.
            WorldEvent::CanonRecorded { entities, .. } => entities.first().copied().unwrap_or(GOD),
            // Typed state names no actor either: observers run as GOD and
            // decode their own subjects from the carried keys.
            WorldEvent::ActionStateChanged { .. } => GOD,
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
    /// The feed id (e.g. `"tactics.map"`).
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
/// (WoD perception modifiers, tactics to-hit modifiers) read, never mutate.
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

/// The reserved environment-query seam (PLAN's tactics seam list; design doc
/// 04 §10.5): rulesets read weather snapshots through this trait, never
/// through the weather module directly. The weather module (M5) registers
/// the single provider via [`Registrar::weather_query`]; consumers reach it
/// via [`Registry::weather_query`]. `None` means no data (indoors, no
/// provider, unzoned room).
pub trait WeatherQuery: Send + Sync {
    /// The current weather at `room` (resolved through its zone).
    fn current(&self, w: &World, room: ObjId) -> Option<WxSnapshot>;
}

/// An interface ability (doc 14 §4.6; plan §3.7, S6): an ability whose provider
/// is a game system rather than a ruleset sheet, fronted by an NPC in
/// conversation and invoked through the same `UseAbility` seam. Each one
/// either reads a projection the **asker** could read with a command, or
/// writes a proposal record the asker owns; the NPC holds no authority of its
/// own. `command` names the command that does the same thing, for the
/// fallback lines when the model is off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterfaceAbility {
    /// The tool name the model sees (`atlas.describe`).
    pub name: SmolStr,
    /// What it does, for the model.
    pub description: String,
    /// A JSON-Schema-ish parameter description.
    pub params_schema: serde_json::Value,
    /// The command that does the same thing (`+atlas`).
    pub command: SmolStr,
}

/// Who an interface ability runs for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityCall {
    /// The NPC fronting the ability.
    pub npc: ObjId,
    /// The speaking player of the stimulus: every read is filtered as this
    /// character's own command would be, every write is theirs.
    pub asker: ObjId,
}

/// The most bytes an interface reply carries.
pub const MAX_INTERFACE_REPLY_BYTES: usize = 4_096;

/// Who hears an interface reply (S8.2, 14a #2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyAudience {
    /// The asker alone: the NPC answers them and no one else hears. The
    /// default, because the provider read as the asker — what the read kept
    /// private, the delivery keeps private.
    #[default]
    Asker,
    /// The room: the NPC says it aloud. A provider's explicit call, for an
    /// answer anyone standing there could have had.
    Room,
}

/// What an interface ability answered: text the NPC speaks, and to whom.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceReply {
    /// The answer, as the asker's own command would print it.
    pub text: String,
    /// Who hears it.
    #[serde(default)]
    pub audience: ReplyAudience,
}

impl InterfaceReply {
    /// A reply for the asker alone, bounded to [`MAX_INTERFACE_REPLY_BYTES`]
    /// on a character edge.
    #[must_use]
    pub fn text(text: impl Into<String>) -> InterfaceReply {
        let mut text: String = text.into();
        if text.len() > MAX_INTERFACE_REPLY_BYTES {
            let mut cut = MAX_INTERFACE_REPLY_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push('…');
        }
        InterfaceReply {
            text,
            audience: ReplyAudience::Asker,
        }
    }

    /// A reply for the asker alone, from lines.
    #[must_use]
    pub fn lines(lines: &[String]) -> InterfaceReply {
        InterfaceReply::text(lines.join("\n"))
    }

    /// A reply the NPC says aloud: the provider's word that anyone in the room
    /// could have had this answer.
    #[must_use]
    pub fn public(text: impl Into<String>) -> InterfaceReply {
        InterfaceReply::text(text).for_room()
    }

    /// A reply said aloud, from lines.
    #[must_use]
    pub fn public_lines(lines: &[String]) -> InterfaceReply {
        InterfaceReply::lines(lines).for_room()
    }

    /// The same reply, for the room.
    #[must_use]
    pub fn for_room(mut self) -> InterfaceReply {
        self.audience = ReplyAudience::Room;
        self
    }
}

/// Why an interface ability refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InterfaceError {
    /// No such ability on this provider.
    #[error("unknown ability: {0}")]
    Unknown(String),
    /// The input was not what the ability takes.
    #[error("invalid input: {0}")]
    Invalid(String),
    /// The asker may not have that, in the words their own command would use.
    #[error("{0}")]
    Refused(String),
}

/// A provider of interface abilities, registered by a game system under an
/// id an NPC's profile names (`interfaces: ["atlas"]`). `invoke` runs on the
/// world loop with the command context and the [`AbilityCall`] naming the
/// asker; a provider reads with the asker's visibility and writes only what
/// the asker owns.
pub trait InterfaceAbilityProvider: Send + Sync {
    /// The abilities this provider fronts.
    fn abilities(&self) -> Vec<InterfaceAbility>;

    /// Invoke `name` with `input` for `call`.
    ///
    /// # Errors
    ///
    /// [`InterfaceError`].
    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<InterfaceReply, InterfaceError>;
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

    /// Answer one query with the live owner-scoped authoritative snapshot.
    ///
    /// Existing handlers remain source-compatible through the default
    /// delegation. A handler that projects a migrated typed family overrides
    /// this method and reads only handles owned by its registered module.
    fn query_with_context(
        &self,
        context: &HookReadContext<'_>,
        actor: ObjId,
        params: &serde_json::Value,
    ) -> Result<OutputEvent, RichText> {
        self.query(context.world(), actor, params)
    }
}

/// Read-only, bounded recovery diagnostics contributed by a mechanics-owning
/// module. The registrar stamps the owner when reports are collected, so a
/// reporter cannot accidentally attribute its incidents to another module.
pub trait RecoveryReporter: Send + Sync {
    /// Snapshot current lifecycle counts and bounded incident/migration detail.
    fn report(&self, world: &World) -> crate::mechanics::ModuleBootReport;
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

// ----------------------------------------------------------------------
// H1 — the module store-read seam
// ----------------------------------------------------------------------

/// What one [`Effect::StoreRead`](crate::engine::Effect::StoreRead) came back
/// with (H1). Carried across the world channel as an encoded `String` in
/// [`WorldCommand::StoreReadDone`](crate::engine::WorldCommand::StoreReadDone)
/// and decoded by [`StoreReadResult::from_payload`] before the module's
/// [`StoreReadCompletion`] sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum StoreReadResult {
    /// Every requested key was looked up. `docs` holds the present ones in
    /// request order; `absent` the keys the store had no document for.
    Docs {
        /// `(key, document)` for each key that existed.
        docs: Vec<(SmolStr, serde_json::Value)>,
        /// The requested keys that did not exist.
        absent: Vec<SmolStr>,
    },
    /// The read did not complete (a store error, a timeout, an undecodable
    /// completion). Nothing is returned; the requester may ask again.
    Failed(String),
    /// The documents would have exceeded [`MAX_STORE_READ_BYTES`]; nothing is
    /// returned. `bytes` is the count observed when the ceiling was crossed,
    /// not the full size.
    Oversized {
        /// Encoded bytes observed before the limit was crossed.
        bytes: usize,
    },
}

/// The wire form of [`StoreReadResult`] — externally tagged, strict.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum StoreReadPayload {
    Docs {
        docs: Vec<(SmolStr, serde_json::Value)>,
        absent: Vec<SmolStr>,
    },
    Failed {
        detail: String,
    },
    Oversized {
        bytes: usize,
    },
}

impl StoreReadResult {
    /// Encode for the world channel. Infallible: every variant is plain data,
    /// and a `Value` that somehow fails to serialize becomes a `Failed`.
    #[must_use]
    pub fn to_payload(&self) -> String {
        let wire = match self {
            StoreReadResult::Docs { docs, absent } => StoreReadPayload::Docs {
                docs: docs.clone(),
                absent: absent.clone(),
            },
            StoreReadResult::Failed(detail) => StoreReadPayload::Failed {
                detail: detail.clone(),
            },
            StoreReadResult::Oversized { bytes } => StoreReadPayload::Oversized { bytes: *bytes },
        };
        serde_json::to_string(&wire).unwrap_or_else(|e| {
            serde_json::to_string(&StoreReadPayload::Failed {
                detail: format!("store read result could not be encoded: {e}"),
            })
            .expect("a plain string payload always encodes")
        })
    }

    /// Decode from the world channel. A malformed payload decodes to
    /// [`StoreReadResult::Failed`] rather than being dropped, so a handler
    /// always hears back for a correlation it is waiting on.
    #[must_use]
    pub fn from_payload(payload: &str) -> StoreReadResult {
        match serde_json::from_str::<StoreReadPayload>(payload) {
            Ok(StoreReadPayload::Docs { docs, absent }) => StoreReadResult::Docs { docs, absent },
            Ok(StoreReadPayload::Failed { detail }) => StoreReadResult::Failed(detail),
            Ok(StoreReadPayload::Oversized { bytes }) => StoreReadResult::Oversized { bytes },
            Err(e) => StoreReadResult::Failed(format!("undecodable store read payload: {e}")),
        }
    }
}

/// Typed re-entry of a module's deferred store read (H1;
/// [`Effect::StoreRead`](crate::engine::Effect::StoreRead) →
/// [`WorldCommand::StoreReadDone`](crate::engine::WorldCommand::StoreReadDone)).
/// The driving loop performs the read off the world loop against the store
/// handle it holds — `mush-core` never does — and re-enters with the result;
/// the engine routes it here, in the world task, with a full [`Ctx`]
/// carrying the requesting module's identity and the requesting actor.
///
/// **Re-validate against current world state**: the actor may have gone, the
/// scene may have closed. A read is a **view**: the handler renders and may
/// update its own RAM bookkeeping (a pending-read watermark), and it never
/// needs to persist. A completion the world cannot apply (a mode refusal, a
/// vanished actor) is dropped with a log and never retried (D-H-3).
pub trait StoreReadCompletion: Send + Sync {
    /// Apply one read result. `correlation` echoes what
    /// [`Ctx::read_store`] returned; `ctx.actor` is the requesting actor.
    fn on_read(&self, ctx: &mut Ctx<'_>, correlation: u64, result: StoreReadResult);
}

/// Why [`Ctx::capture_decision`] could not even queue a capture (H2a).
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum DecisionCaptureRequestError {
    #[error("capture_decision needs a module context")]
    NoModule,
    #[error("the module's name is not a canonical mechanics owner: {0}")]
    Owner(String),
}

/// Why [`Ctx::read_store`] refused to queue a read. Every variant is decided
/// before anything is deferred, so a refused read leaves no trace.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreReadError {
    /// The context carries no module identity (a core command), so there is
    /// no namespace to read.
    #[error("only a module may read the store; this context has no module identity")]
    NoModule,
    /// More keys than [`MAX_STORE_READ_KEYS`].
    #[error("a store read names at most {max} keys ({requested} requested)")]
    TooManyKeys {
        /// Keys requested.
        requested: usize,
        /// The ceiling.
        max: usize,
    },
    /// A read of nothing is a programming error, not a no-op.
    #[error("a store read names at least one key")]
    NoKeys,
    /// An empty key cannot name a document.
    #[error("a store read key must not be empty")]
    EmptyKey,
    /// A key past [`MAX_MODULE_KEY_BYTES`] cannot exist in the store.
    #[error("store read key is {bytes} bytes, exceeding the {max}-byte limit")]
    KeyTooLong {
        /// UTF-8 encoded key length.
        bytes: usize,
        /// The ceiling.
        max: usize,
    },
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
/// dispatch — mail compose, chargen wizards, the tactics unit console.
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
    /// The slot type's [`std::any::type_name`], carried because a `TypeId`
    /// is an opaque hash: every report about a slot — the ownership census
    /// (WS03-S8), the boot warn for an unowned slot — has to be able to
    /// NAME the type it is talking about.
    type_name: &'static str,
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

    /// The declared slot's type name (diagnostics only).
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        self.type_name
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
    /// The slot type's name, kept so a report can name it (see
    /// [`StateInit::type_name`]).
    type_name: &'static str,
}

/// One installed state slot, as the ownership census sees it: what type the
/// slot holds and which module — if any — declared it.
///
/// A slot with `owner: None` was seeded through
/// [`World::module_state_insert`](crate::world::World::module_state_insert)
/// rather than declared through [`Registrar::state`]; it stays open to every
/// module — there is no owner to refuse for — and the engine names it in a
/// boot `warn!` rather than refusing it.
#[derive(Clone, Debug)]
pub struct StateSlotInfo {
    /// The slot's type.
    pub type_id: TypeId,
    /// That type's [`std::any::type_name`].
    pub type_name: &'static str,
    /// The module that declared the slot, if one did.
    pub owner: Option<SmolStr>,
}

/// A module-facing state-slot mutation that was **refused**: `requester`
/// asked to mutate a slot `owner` declared.
///
/// Returned by [`Ctx::try_state_mut`]. Nothing panics on one — the world task
/// must never panic — and nothing is written: the refusal is total, and the
/// engine logs it once per (requester, slot) and records it on the world
/// ([`World::refused_state_writes`](crate::world::World::refused_state_writes)).
///
/// Only the module-facing surface refuses. The engine accessor
/// [`World::module_state_mut`](crate::world::World::module_state_mut) carries
/// no module identity and cannot, which is why the cross-owner reaches that go
/// through it are pinned by `mush-contract-tests`' `state-slot-owners.toml`
/// instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateSlotDenied {
    /// The module whose handler asked for the slot.
    pub requester: SmolStr,
    /// The module that declared it through [`Registrar::state`].
    pub owner: SmolStr,
    /// The slot type's [`std::any::type_name`].
    pub slot: &'static str,
}

impl fmt::Display for StateSlotDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "module {:?} may not mutate the state slot {} — it belongs to module {:?}",
            self.requester, self.slot, self.owner
        )
    }
}

impl std::error::Error for StateSlotDenied {}

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
                            type_name: v.type_name,
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
                type_name: std::any::type_name::<T>(),
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
                type_name: init.type_name,
            },
        );
    }

    /// Every installed slot, as `(TypeId, type name)`, in no particular
    /// order.
    pub fn slots(&self) -> impl Iterator<Item = (TypeId, &'static str)> + '_ {
        self.slots.iter().map(|(&id, slot)| (id, slot.type_name))
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

/// One declared cross-namespace write delegation (WS03-A1): module `writer`
/// may persist into namespace `ns`, which belongs to another module.
///
/// Declared through [`Registrar::persist_delegation`] by one of the two
/// modules the pair names — the writer, or the namespace's owner — because
/// only they can say why the write is legitimate. `mush-contract-tests`'
/// `module-ns-owners.toml` pins the composed set, so a new delegation cannot
/// arrive without a reviewed row.
#[derive(Clone, Debug)]
pub struct PersistDelegation {
    /// The module whose identity is on the write (`ctx.module`).
    pub writer: SmolStr,
    /// The namespace the write lands in (`op.ns`).
    pub ns: SmolStr,
    /// Which of the two declared it.
    pub declared_by: SmolStr,
    /// How a write gets from that module into that namespace.
    pub reason: &'static str,
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
    /// Canonical module owner -> its read-only recovery diagnostics.
    recovery_reporters:
        std::collections::BTreeMap<crate::mechanics::OwnerId, Arc<dyn RecoveryReporter>>,
    job_queues: Vec<JobQueueSpec>,
    script_apis: Vec<(SmolStr, SmolStr)>,
    migrations: HashMap<SmolStr, Vec<Migration>>,
    /// migration namespace → the one module that owns its migration set.
    migration_owners: HashMap<SmolStr, SmolStr>,
    /// WS03-A1: persistence namespace → the module that owns it. Filled in
    /// once, at composition, from the roster's module names and the
    /// migration owners; a namespace nobody claims is absent, not empty.
    namespace_owners: HashMap<SmolStr, SmolStr>,
    /// WS03-A1: the declared cross-namespace write delegations, one row per
    /// `(writer, namespace)` pair. Half a dozen entries at most, so a scan
    /// beats a hash — and the scan only ever runs on a mismatch.
    persist_delegations: Vec<PersistDelegation>,
    /// The single registered weather provider (the reserved tactics seam).
    weather: Option<(SmolStr, Arc<dyn WeatherQuery>)>,
    /// S6: interface-ability providers by id → (owning module, provider).
    interface_providers:
        std::collections::BTreeMap<SmolStr, (SmolStr, Arc<dyn InterfaceAbilityProvider>)>,
    /// module name → its async-completion handler (the LLM re-entry seam).
    async_completions: HashMap<SmolStr, Arc<dyn AsyncCompletion>>,
    /// module name → its request-completion handler (interface plan I10: the
    /// server-initiated Request/Respond re-entry seam).
    request_completions: HashMap<SmolStr, Arc<dyn crate::request::RequestCompletion>>,
    /// Discord interaction handlers by `custom_id` prefix (programme D3, 03
    /// §7a.4): `(prefix, module, handler)`, longest prefix wins.
    discord_interactions: Vec<(SmolStr, SmolStr, Arc<dyn DiscordInteractionHandler>)>,
    /// The one module that speaks relayed lines onto channels (programme D4):
    /// `(module, handler)`.
    relay_speaker: Option<(SmolStr, Arc<dyn RelaySpeaker>)>,
    /// module name → its store-read completion handler (the H1 read seam).
    store_read_completions: HashMap<SmolStr, Arc<dyn StoreReadCompletion>>,
    /// module name → its capture action and decision capture sink (the H2a
    /// capture seam).
    decision_capture_sinks: HashMap<
        SmolStr,
        (
            crate::mechanics::DefinitionRef,
            crate::decision::DecisionCaptureSink,
        ),
    >,
    state_inits: Vec<StateInit>,
}

impl fmt::Debug for ModuleTables {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ModuleTables(ns:{} markers:{} ticks:{} observers:{} recovery:{})",
            self.attr_ns.len(),
            self.markers.len(),
            self.ticks.len(),
            self.observers.len(),
            self.recovery_reporters.len()
        )
    }
}

impl Registry {
    /// Freeze the K1 definition/bridge/migration registries and K2 action providers.
    /// Called exactly once by [`Engine::compose_modules`](crate::Engine::compose_modules)
    /// after every module has registered and before persisted world data loads.
    pub(crate) fn freeze_mechanics(
        &mut self,
        module_owners: &BTreeSet<crate::mechanics::OwnerId>,
    ) -> Result<(), ModError> {
        if module_owners.len() > crate::mechanics::MAX_MECHANICS_OWNERS {
            return Err(ModError::msg(format!(
                "mechanics owner roster has {} entries; maximum is {}",
                module_owners.len(),
                crate::mechanics::MAX_MECHANICS_OWNERS
            )));
        }
        if self.definitions.is_some()
            || self.bridges.is_some()
            || self.definition_migrations.is_some()
            || self.action_providers.is_some()
            || self.revision_validators.is_some()
            || self.authority_guard_validators.is_some()
            || self.state_families.is_some()
            || self.clock_providers.is_some()
            || self.resource_effect_providers.is_some()
            || self.schedule_sources.is_some()
            || self.decision_sources.is_some()
            || self.mechanics_owner_readiness.is_some()
        {
            return Err(ModError::msg("mechanics registries were already frozen"));
        }
        let mut definitions = std::mem::take(&mut self.definition_builder)
            .freeze()
            .map_err(|e| ModError::msg(format!("definition registry: {e}")))?;
        let owner_readiness = crate::mechanics::MechanicalOwnerReadiness::for_owners(module_owners);
        definitions.bind_owner_readiness(owner_readiness.clone());
        let bridges = std::mem::take(&mut self.bridge_builder)
            .freeze_for_owners_with_readiness(&definitions, module_owners, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("bridge registry: {e}")))?;
        let action_providers = std::mem::take(&mut self.action_provider_builder)
            .freeze_for_owners_with_readiness(&definitions, module_owners, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("action provider registry: {e}")))?;
        let state_families = std::mem::take(&mut self.state_family_builder)
            .freeze_for_owners_with_readiness(module_owners, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("state family registry: {e}")))?;
        let clock_providers = std::mem::take(&mut self.clock_provider_builder)
            .freeze_for_owners_with_readiness(module_owners, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("clock provider registry: {e}")))?;
        let resource_effect_providers = std::mem::take(&mut self.resource_effect_provider_builder)
            .freeze_for_composition(
                &definitions,
                module_owners,
                &state_families,
                owner_readiness.clone(),
            )
            .map_err(|e| ModError::msg(format!("resource/effect provider registry: {e}")))?;
        let schedule_sources = std::mem::take(&mut self.schedule_source_builder)
            .freeze_for_composition(
                module_owners,
                &action_providers,
                &state_families,
                owner_readiness.clone(),
            )
            .map_err(|e| ModError::msg(format!("schedule source registry: {e}")))?;
        let decision_sources = std::mem::take(&mut self.decision_source_builder)
            .freeze_for_composition(
                module_owners,
                &action_providers,
                &state_families,
                owner_readiness.clone(),
            )
            .map_err(|e| ModError::msg(format!("decision source registry: {e}")))?;
        let revision_validators = std::mem::take(&mut self.revision_validator_builder)
            .freeze_for_owners_with_readiness(module_owners, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("revision validator registry: {e}")))?;
        let authority_guard_validators =
            std::mem::take(&mut self.authority_guard_validator_builder)
                .freeze_for_owners_with_readiness(module_owners, owner_readiness.clone())
                .map_err(|e| ModError::msg(format!("authority validator registry: {e}")))?;
        let definition_migrations = std::mem::take(&mut self.definition_migration_builder)
            .freeze_for_composition(&definitions, owner_readiness.clone())
            .map_err(|e| ModError::msg(format!("definition migration registry: {e}")))?;
        self.definitions = Some(definitions);
        self.mechanics_owner_readiness = Some(owner_readiness);
        self.bridges = Some(bridges);
        self.action_providers = Some(action_providers);
        self.revision_validators = Some(revision_validators);
        self.authority_guard_validators = Some(authority_guard_validators);
        self.state_families = Some(state_families);
        self.clock_providers = Some(clock_providers);
        self.resource_effect_providers = Some(resource_effect_providers);
        self.schedule_sources = Some(schedule_sources);
        self.decision_sources = Some(decision_sources);
        self.definition_migrations = Some(definition_migrations);
        Ok(())
    }

    /// The immutable exact definition registry, once module composition has
    /// completed. `None` is possible only on a bare, uncomposed [`Registry`].
    #[must_use]
    pub fn definitions(&self) -> Option<&crate::mechanics::DefinitionRegistry> {
        self.definitions.as_ref()
    }

    /// Permanently disable executable mechanics owned by one composed module.
    /// The first incident wins and the transition is monotonic for this process.
    pub fn freeze_mechanics_owner(
        &self,
        owner: &crate::mechanics::OwnerId,
        incident: crate::mechanics::IncidentId,
        reason: crate::mechanics::RecoveryReason,
    ) -> Result<
        crate::mechanics::MechanicalOwnerFreeze,
        crate::mechanics::MechanicalOwnerReadinessError,
    > {
        self.mechanics_owner_readiness
            .as_ref()
            .ok_or(crate::mechanics::MechanicalOwnerReadinessError::RegistryUnavailable)?
            .freeze(owner, incident, reason)
    }

    /// Return the incident holding an owner, or `None` when it remains ready.
    pub fn mechanics_owner_freeze(
        &self,
        owner: &crate::mechanics::OwnerId,
    ) -> Result<
        Option<crate::mechanics::MechanicalOwnerFreeze>,
        crate::mechanics::MechanicalOwnerReadinessError,
    > {
        self.mechanics_owner_readiness
            .as_ref()
            .ok_or(crate::mechanics::MechanicalOwnerReadinessError::RegistryUnavailable)?
            .freeze_for(owner)
    }

    /// Fail closed unless an owner belongs to the frozen composition roster and
    /// has not encountered an integrity incident.
    pub fn require_mechanics_owner_ready(
        &self,
        owner: &crate::mechanics::OwnerId,
    ) -> Result<(), crate::mechanics::MechanicalOwnerReadinessError> {
        self.mechanics_owner_readiness
            .as_ref()
            .ok_or(crate::mechanics::MechanicalOwnerReadinessError::RegistryUnavailable)?
            .require_ready(owner)
    }

    /// The immutable exact cross-owner bridge registry after composition.
    #[must_use]
    pub fn bridges(&self) -> Option<&crate::mechanics::BridgeRegistry> {
        self.bridges.as_ref()
    }

    /// Pure definition migrations available for dry-run planning. These are
    /// deliberately separate from the legacy eager namespace migrations.
    #[must_use]
    pub fn definition_migrations(&self) -> Option<&crate::mechanics::DefinitionMigrationRegistry> {
        self.definition_migrations.as_ref()
    }

    /// Exact, immutable action-provider lookup after module composition.
    #[must_use]
    pub fn action_providers(&self) -> Option<&crate::action::ActionProviderRegistry> {
        self.action_providers.as_ref()
    }

    /// Exact, immutable revision-validator lookup after module composition.
    #[must_use]
    pub fn revision_validators(&self) -> Option<&crate::action::RevisionValidatorRegistry> {
        self.revision_validators.as_ref()
    }

    /// Exact, immutable authority-validator lookup after module composition.
    #[must_use]
    pub fn authority_guard_validators(
        &self,
    ) -> Option<&crate::action::AuthorityGuardValidatorRegistry> {
        self.authority_guard_validators.as_ref()
    }

    /// Immutable owner-stamped state-family registry after composition.
    #[must_use]
    pub fn state_families(&self) -> Option<&crate::action_state::StateFamilyRegistry> {
        self.state_families.as_ref()
    }

    /// Immutable owner-stamped clock-provider registry after composition.
    #[must_use]
    pub fn clock_providers(&self) -> Option<&crate::clock_provider::ClockProviderRegistry> {
        self.clock_providers.as_ref()
    }

    /// Immutable resource/effect read adapters after module composition.
    #[must_use]
    pub fn resource_effect_providers(
        &self,
    ) -> Option<&crate::resource_provider::ResourceEffectProviderRegistry> {
        self.resource_effect_providers.as_ref()
    }

    /// Immutable declarative durable schedule sources after composition.
    #[must_use]
    pub fn schedule_sources(&self) -> Option<&crate::schedule_source::ScheduleSourceRegistry> {
        self.schedule_sources.as_ref()
    }

    /// Immutable delayed-decision sources after module composition.
    #[must_use]
    pub fn decision_sources(&self) -> Option<&crate::decision_source::DecisionSourceRegistry> {
        self.decision_sources.as_ref()
    }

    /// Start a server-owned guard bundle bound to this exact frozen module
    /// composition. A bare or not-yet-composed registry fails closed.
    pub fn server_guard_set_builder(
        &self,
        revisions: crate::action::RevisionSet,
    ) -> Result<crate::action::ServerGuardSetBuilder, crate::action::GuardSetBuildError> {
        let readiness = self
            .mechanics_owner_readiness
            .as_ref()
            .ok_or(crate::action::GuardSetBuildError::RegistryUnavailable)?;
        Ok(crate::action::ServerGuardSetBuilder::new(
            revisions,
            readiness.clone(),
        ))
    }

    /// Start a deterministic trigger queue bound to this exact frozen module
    /// composition. Provider tokens from any other composition are rejected.
    pub fn causal_trigger_queue(
        &self,
        root_request: crate::action::RequestUid,
    ) -> Result<crate::action::CausalTriggerQueue, crate::action::CascadeError> {
        let readiness = self
            .mechanics_owner_readiness
            .as_ref()
            .ok_or(crate::action::CascadeError::RegistryUnavailable)?;
        Ok(crate::action::CausalTriggerQueue::new(
            root_request,
            readiness.clone(),
        ))
    }

    /// One deployment fingerprint over all frozen mechanics registries.
    #[must_use]
    pub fn mechanics_fingerprint(&self) -> Option<crate::mechanics::DefinitionDigest> {
        let definitions = self.definitions.as_ref()?;
        let bridges = self.bridges.as_ref()?;
        let migrations = self.definition_migrations.as_ref()?;
        let actions = self.action_providers.as_ref()?;
        let revisions = self.revision_validators.as_ref()?;
        let authority = self.authority_guard_validators.as_ref()?;
        let state_families = self.state_families.as_ref()?;
        let clock_providers = self.clock_providers.as_ref()?;
        let resource_effect_providers = self.resource_effect_providers.as_ref()?;
        let schedule_sources = self.schedule_sources.as_ref()?;
        let decision_sources = self.decision_sources.as_ref()?;
        crate::mechanics::DefinitionDigest::for_canonical_value(
            "mechanics_registry.v8",
            &serde_json::json!({
                "definitions": definitions.fingerprint().to_hex(),
                "bridges": bridges.fingerprint().to_hex(),
                "migrations": migrations.fingerprint().to_hex(),
                "actions": actions.fingerprint().to_hex(),
                "revision_validators": revisions.fingerprint().to_hex(),
                "authority_validators": authority.fingerprint().to_hex(),
                "state_families": state_families.fingerprint().to_hex(),
                "clock_providers": clock_providers.fingerprint().to_hex(),
                "resource_effect_providers": resource_effect_providers.fingerprint().to_hex(),
                "schedule_sources": schedule_sources.fingerprint().to_hex(),
                "decision_sources": decision_sources.fingerprint().to_hex(),
            }),
        )
        .ok()
    }

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

    /// Collect every module's bounded recovery report in canonical owner
    /// order. Raw quarantined payloads are intentionally never exposed here.
    #[must_use]
    pub fn recovery_reports(&self, world: &World) -> Vec<crate::mechanics::ModuleBootReport> {
        self.mods
            .recovery_reporters
            .iter()
            .map(|(owner, reporter)| {
                let mut report = reporter.report(world);
                let incident_details = u64::try_from(report.incidents.len()).unwrap_or(u64::MAX);
                let migration_details =
                    u64::try_from(report.migration_plans.len()).unwrap_or(u64::MAX);
                report.incident_count = report.incident_count.max(incident_details);
                report.migration_plan_count = report.migration_plan_count.max(migration_details);
                if report.incidents.len() > crate::mechanics::MAX_BOOT_REPORT_DETAILS {
                    report
                        .incidents
                        .truncate(crate::mechanics::MAX_BOOT_REPORT_DETAILS);
                    report.details_truncated = true;
                }
                if report.migration_plans.len() > crate::mechanics::MAX_BOOT_REPORT_DETAILS {
                    report
                        .migration_plans
                        .truncate(crate::mechanics::MAX_BOOT_REPORT_DETAILS);
                    report.details_truncated = true;
                }
                report.owner = owner.clone();
                // Incident attribution is registrar-owned just like the
                // top-level report. Reasons may legitimately name a different
                // provider, but an incident itself may not spoof its owner.
                for incident in &mut report.incidents {
                    incident.owner = owner.clone();
                }
                report
            })
            .collect()
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

    /// The module that owns the persistence namespace `ns` (WS03-A1), if the
    /// composition claims it.
    ///
    /// A module's [`GameModule::name`] **is** its namespace, so the roster's
    /// names are the bulk of this map; a namespace no module is named after
    /// belongs to whichever module first registered its migrations
    /// ([`Registrar::migrations`]' first-registrar rule). A namespace nobody
    /// claims — an engine-owned one such as the world timeline, or a test
    /// rig's — is simply absent.
    #[must_use]
    pub fn namespace_owner(&self, ns: &str) -> Option<&SmolStr> {
        self.mods.namespace_owners.get(ns)
    }

    /// The declared delegation letting `writer` persist into `ns`, if there
    /// is one (WS03-A1). Consulted only on a namespace mismatch.
    #[must_use]
    pub fn persist_delegation(&self, writer: &str, ns: &str) -> Option<&PersistDelegation> {
        self.mods
            .persist_delegations
            .iter()
            .find(|d| d.writer == writer && d.ns == ns)
    }

    /// Every declared cross-namespace write delegation, in declaration order.
    #[must_use]
    pub fn persist_delegations(&self) -> &[PersistDelegation] {
        &self.mods.persist_delegations
    }

    /// Every claimed persistence namespace with its owning module, sorted.
    #[must_use]
    pub fn namespace_owners(&self) -> Vec<(SmolStr, SmolStr)> {
        let mut owners: Vec<(SmolStr, SmolStr)> = self
            .mods
            .namespace_owners
            .iter()
            .map(|(ns, module)| (ns.clone(), module.clone()))
            .collect();
        owners.sort();
        owners
    }

    /// Record `namespace -> owning module` for the composed roster (WS03-A1).
    ///
    /// Called once by [`Engine::compose_modules`](crate::Engine::compose_modules)
    /// with every module's name, after registration and before the world sees
    /// a single write. A module name always wins over a migration
    /// registration for the same string: the name is the namespace by
    /// definition, and the migration table only names namespaces no module is
    /// called after.
    pub(crate) fn record_namespace_owners<'a>(
        &mut self,
        module_names: impl IntoIterator<Item = &'a str>,
    ) {
        let mut owners: HashMap<SmolStr, SmolStr> = self.mods.migration_owners.clone();
        for name in module_names {
            let name = SmolStr::new(name);
            owners.insert(name.clone(), name);
        }
        self.mods.namespace_owners = owners;
    }

    /// The registered weather provider (the reserved `WeatherQuery` seam),
    /// with its owning module, if one is loaded.
    #[must_use]
    pub fn weather_query(&self) -> Option<(&SmolStr, Arc<dyn WeatherQuery>)> {
        self.mods.weather.as_ref().map(|(m, p)| (m, Arc::clone(p)))
    }

    /// S6: the interface-ability provider registered under `id`, if any.
    #[must_use]
    pub fn interface_provider(&self, id: &str) -> Option<Arc<dyn InterfaceAbilityProvider>> {
        self.mods
            .interface_providers
            .get(id)
            .map(|(_, p)| Arc::clone(p))
    }

    /// S6: every interface-ability provider: `(id, owning module, provider)`.
    pub fn interface_providers(
        &self,
    ) -> impl Iterator<Item = (&SmolStr, &SmolStr, Arc<dyn InterfaceAbilityProvider>)> {
        self.mods
            .interface_providers
            .iter()
            .map(|(id, (m, p))| (id, m, Arc::clone(p)))
    }

    /// The async-completion handler registered by `module`, if any (the
    /// engine routes [`WorldCommand::ModuleAsyncDone`](crate::engine::WorldCommand::ModuleAsyncDone)
    /// here).
    #[must_use]
    pub fn async_completion(&self, module: &str) -> Option<Arc<dyn AsyncCompletion>> {
        self.mods.async_completions.get(module).map(Arc::clone)
    }

    /// This module's registered
    /// [`RequestCompletion`](crate::request::RequestCompletion), if any
    /// (interface plan I10).
    #[must_use]
    pub fn request_completion(
        &self,
        module: &str,
    ) -> Option<Arc<dyn crate::request::RequestCompletion>> {
        self.mods.request_completions.get(module).map(Arc::clone)
    }

    /// The module that speaks relayed lines, with its handler (programme D4).
    #[must_use]
    pub fn relay_speaker(&self) -> Option<(SmolStr, Arc<dyn RelaySpeaker>)> {
        self.mods
            .relay_speaker
            .as_ref()
            .map(|(m, h)| (m.clone(), Arc::clone(h)))
    }

    /// The Discord interaction handler whose prefix is the longest match for
    /// `custom_id`, with its module (programme D3).
    #[must_use]
    pub fn discord_interaction_handler(
        &self,
        custom_id: &str,
    ) -> Option<(SmolStr, Arc<dyn DiscordInteractionHandler>)> {
        self.mods
            .discord_interactions
            .iter()
            .filter(|(prefix, _, _)| custom_id.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _, _)| prefix.len())
            .map(|(_, module, handler)| (module.clone(), Arc::clone(handler)))
    }
    /// The store-read completion handler registered by `module`, if any (the
    /// engine routes [`WorldCommand::StoreReadDone`](crate::engine::WorldCommand::StoreReadDone)
    /// here — H1).
    #[must_use]
    pub fn store_read_completion(&self, module: &str) -> Option<Arc<dyn StoreReadCompletion>> {
        self.mods.store_read_completions.get(module).map(Arc::clone)
    }

    /// The decision capture sink `module` (a mechanics owner) registered, if
    /// any (H2a). The driving loop arms it with a built plan; the owner's
    /// capture action provider drains it while preparing that plan's bundle.
    #[must_use]
    pub fn decision_capture_sink(
        &self,
        module: &str,
    ) -> Option<(
        crate::mechanics::DefinitionRef,
        crate::decision::DecisionCaptureSink,
    )> {
        self.mods.decision_capture_sinks.get(module).cloned()
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
/// Registered as `Box<dyn GameModule>` **before** persisted world load via
/// [`Engine::compose_modules`](crate::engine::Engine::compose_modules).
pub trait GameModule: Send {
    /// Stable module id — also the persistence namespace and the identity
    /// commands registered here carry (`ctx.module`). It must already be a
    /// canonical [`OwnerId`](crate::mechanics::OwnerId): lowercase ASCII
    /// letters, digits, `_`, and non-empty `.`-separated segments.
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
    /// Pure over `kv` — called after coordinator-owned typed state has passed
    /// atomic boot hydration and before [`GameModule::on_world_loaded`], so a
    /// module whose stream keys are derived from its docs (comsys `hist:{id}`
    /// per `chan:{id}`) can name them without the driver knowing its layout.
    /// Invalid persisted metadata must return an error here, before the driver
    /// performs any stream read.
    /// Default: none.
    fn boot_stream_requests(
        &self,
        _kv: &[(SmolStr, serde_json::Value)],
    ) -> Result<Vec<(SmolStr, u32)>, ModError> {
        Ok(Vec::new())
    }
}

/// The registration surface handed to [`GameModule::register`]: an
/// identity-carrying view over the one [`Registry`]. Commands and alias
/// stages go straight into the existing tables; everything else into the
/// registry's [`ModuleTables`].
pub struct Registrar<'a> {
    module: SmolStr,
    reg: &'a mut Registry,
    mechanics_authority: MechanicsRegistrationAuthority,
    command_compatibility: Option<crate::cmd::CommandCompatibility>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MechanicsRegistrationAuthority {
    Legacy,
    ModuleComposition,
}

impl<'a> Registrar<'a> {
    /// A registrar acting as `module` over `reg` for legacy/non-mechanical
    /// registration surfaces.
    ///
    /// Mechanics registration is deliberately unavailable through this public
    /// constructor: otherwise any caller holding a bare [`Registry`] could
    /// claim the name of a later composed module and pre-populate its
    /// owner-stamped definitions or executable callbacks. Mechanics are
    /// admitted only by the registrar created around the actual
    /// [`GameModule::register`] composition callback.
    ///
    /// ```compile_fail
    /// use mush_core::{Registrar, Registry};
    /// let mut registry = Registry::new();
    /// let _ = Registrar::for_composition(&mut registry, "spoofed");
    /// ```
    pub fn new(reg: &'a mut Registry, module: &str) -> Registrar<'a> {
        Registrar {
            module: SmolStr::new(module),
            reg,
            mechanics_authority: MechanicsRegistrationAuthority::Legacy,
            // The public constructor is retained for old host/test assembly.
            // Real module composition starts unclassified below.
            command_compatibility: Some(crate::cmd::CommandCompatibility::LegacyPostImage),
        }
    }

    /// Create the capability-bearing registrar used only while composing the
    /// concrete module whose `name()` supplied `module`.
    pub(crate) fn for_composition(reg: &'a mut Registry, module: &str) -> Registrar<'a> {
        Registrar {
            module: SmolStr::new(module),
            reg,
            mechanics_authority: MechanicsRegistrationAuthority::ModuleComposition,
            command_compatibility: None,
        }
    }

    fn require_mechanics_registration_authority(&self) -> Result<(), ModError> {
        if self.mechanics_authority != MechanicsRegistrationAuthority::ModuleComposition {
            return Err(ModError::msg(format!(
                "module {}: mechanics registration requires the Engine::compose_modules callback",
                self.module
            )));
        }
        Ok(())
    }

    /// The registering module's id.
    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Register one immutable rules definition owned by this module. The
    /// descriptor cannot spoof an owner or digest: this registrar stamps the
    /// former and the shared builder computes the latter.
    pub fn definition(
        &mut self,
        descriptor: crate::mechanics::DefinitionDescriptor,
    ) -> Result<crate::mechanics::DefinitionRef, ModError> {
        self.require_mechanics_registration_authority()?;
        if self.reg.definitions.is_some() {
            return Err(ModError::msg(format!(
                "module {}: definition registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .definition_builder
            .register_provider_definition(owner, descriptor)
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: definition registration: {e}",
                    self.module
                ))
            })
    }

    /// Register the one semantic validator for a definition kind owned by
    /// this module.
    ///
    /// The shared registry validates canonical identity, bounds, and exact
    /// dependency availability. This callback is the typed provider's half of
    /// that contract: it must strictly decode and completely validate each
    /// payload of `kind`, including checking that references derived from the
    /// decoded body exactly equal the descriptor's declared references.
    /// Composition fails if a kind registered through [`Self::definition`]
    /// has no validator, if this pair is registered twice, or if validation
    /// rejects any definition.
    pub fn definition_validator<V>(
        &mut self,
        kind: crate::mechanics::DefinitionKind,
        validator: V,
    ) -> Result<(), ModError>
    where
        V: crate::mechanics::DefinitionValidator + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.definitions.is_some() {
            return Err(ModError::msg(format!(
                "module {}: definition registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .definition_builder
            .register_provider_validator(owner, kind, Arc::new(validator))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: definition validator registration: {e}",
                    self.module
                ))
            })
    }

    /// Register an object-safe action provider for one exact action definition.
    ///
    /// The registrar stamps the provider owner. The referenced definition must
    /// have kind `action`, the same owner, and an exact registered digest when
    /// module composition freezes.
    pub fn action_provider<P>(
        &mut self,
        action: crate::mechanics::DefinitionRef,
        provider: P,
    ) -> Result<(), ModError>
    where
        P: crate::action::ActionProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.action_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: action provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .action_provider_builder
            .register(owner, action, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: action provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Declare one engine-owned typed state family and receive its opaque
    /// owner-stamped capability.
    ///
    /// The descriptor names the durable key/schema/validation/serialization
    /// contracts. The callbacks are trusted module code, but live values can
    /// be read or mutated only through the returned handle and a bounded
    /// [`ActionTxn`](crate::ActionTxn). No family is installed in the legacy
    /// [`ModuleStateMap`].
    #[allow(clippy::too_many_arguments)]
    pub fn state_family<K, V, KC, VD, VV, VS>(
        &mut self,
        descriptor: crate::action_state::StateFamilyDescriptor,
        key_codec: KC,
        decoder: VD,
        validator: VV,
        serializer: VS,
    ) -> Result<crate::action_state::StateFamilyHandle<K, V>, ModError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
        KC: crate::action_state::StateKeyCodec<K> + 'static,
        VD: crate::action_state::StateValueDecoder<V> + 'static,
        VV: crate::action_state::StateValueValidator<V> + 'static,
        VS: crate::action_state::StateValueSerializer<V> + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.state_families.is_some() {
            return Err(ModError::msg(format!(
                "module {}: state-family registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .state_family_builder
            .register(owner, descriptor, key_codec, decoder, validator, serializer)
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: state-family registration: {e}",
                    self.module
                ))
            })
    }

    /// Delegate this module's typed state families to one other composed
    /// owner, so a provider registered by that owner may read and write them
    /// **inside its own bundle**.
    ///
    /// Declared by the **owning** module only: the registrar stamps the caller
    /// as the family owner, so no module can help itself to another's state.
    /// A delegation is never implicit, never transitive and never symmetric,
    /// at most [`MAX_STATE_FAMILY_DELEGATES`](crate::action_state::MAX_STATE_FAMILY_DELEGATES)
    /// grantees per owner, and it enters the state-family fingerprint — two
    /// deployments differing only in a delegation are not the same deployment.
    ///
    /// It exists for the one shape the tree could not otherwise express: a
    /// single authoritative bundle whose post-images belong to two owners
    /// (E7 cut 3's extraction debits `mush-inventory`'s deposit pool and
    /// advances `mush-industry`'s run in one commit). The persisted namespace
    /// of every write stays the **family owner's**.
    ///
    /// # Errors
    /// [`ModError`] if the registry is frozen, the grantee is not a canonical
    /// owner id, the grantee is this module, or the delegate cap is reached.
    pub fn delegate_state_families(&mut self, grantee: &str) -> Result<(), ModError> {
        self.require_mechanics_registration_authority()?;
        if self.reg.state_families.is_some() {
            return Err(ModError::msg(format!(
                "module {}: state-family registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        let grantee = crate::mechanics::OwnerId::new(grantee).map_err(|e| {
            ModError::msg(format!(
                "module {}: state-family delegate is not a canonical owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .state_family_builder
            .delegate(owner, grantee)
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: state-family delegation: {e}",
                    self.module
                ))
            })
    }

    /// Register a synchronous provider for one exact module-owned clock.
    ///
    /// The registrar stamps the provider owner. At read time the kernel
    /// supplies the requested generation and immutable world/state context;
    /// the provider returns only the scalar value, never a relabelled stamp.
    pub fn clock_provider<P>(
        &mut self,
        kind: crate::clock_provider::ClockProviderKind,
        id: crate::mechanics::CanonicalId,
        provider: P,
    ) -> Result<(), ModError>
    where
        P: crate::clock_provider::ClockProvider + 'static,
    {
        if kind == crate::clock_provider::ClockProviderKind::DomainTick {
            return Err(ModError::msg(
                "dynamic domain clocks must use domain_clock_provider",
            ));
        }
        self.require_mechanics_registration_authority()?;
        if self.reg.clock_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: clock-provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .clock_provider_builder
            .register(owner, kind, id, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: clock provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Register this module's one provider for dynamic domain clocks.
    ///
    /// The registrar stamps the owner once at composition. Runtime domain ids
    /// arrive from `ClockRef::DomainTick`; promoting a domain never mutates the
    /// frozen provider registry.
    pub fn domain_clock_provider<P>(&mut self, provider: P) -> Result<(), ModError>
    where
        P: crate::clock_provider::DomainClockProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.clock_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: clock-provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .clock_provider_builder
            .register_domain(owner, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: domain clock provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Register this module as the deployment's sole provider for dynamic
    /// [`ClockRef::SceneTurn`](crate::clock::ClockRef::SceneTurn) clocks.
    ///
    /// Scene ids are world-global and the clock reference intentionally has
    /// no module-owner field, so composition rejects a second provider rather
    /// than allowing ambiguous routing.
    pub fn scene_clock_provider<P>(&mut self, provider: P) -> Result<(), ModError>
    where
        P: crate::clock_provider::SceneClockProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.clock_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: clock-provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .clock_provider_builder
            .register_scene(owner, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: scene clock provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Register a read-only adapter for one exact resource definition owned
    /// by this module. The definition's stable id becomes the resource id;
    /// composition selects exactly one version for that address.
    pub fn resource_provider<P>(
        &mut self,
        definition: crate::mechanics::DefinitionRef,
        provider: P,
    ) -> Result<(), ModError>
    where
        P: crate::resource_provider::ResourceProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.resource_effect_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: resource/effect provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .resource_effect_provider_builder
            .register_resource(owner, definition, None, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: resource provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Register a resource adapter together with the owner-local
    /// authoritative reservation-book family that gates its spendable value.
    pub fn reservable_resource_provider<P>(
        &mut self,
        definition: crate::mechanics::DefinitionRef,
        reservations: crate::action_state::StateFamilyHandle<
            crate::resource::ReservationBookId,
            crate::resource::ReservationBook,
        >,
        provider: P,
    ) -> Result<(), ModError>
    where
        P: crate::resource_provider::ResourceProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.resource_effect_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: resource/effect provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .resource_effect_provider_builder
            .register_resource(owner, definition, Some(reservations), Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: reservable resource provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Register a read-only adapter for instances of one exact effect
    /// definition owned by this module.
    pub fn effect_provider<P>(
        &mut self,
        definition: crate::mechanics::DefinitionRef,
        provider: P,
    ) -> Result<(), ModError>
    where
        P: crate::resource_provider::EffectProvider + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.resource_effect_providers.is_some() {
            return Err(ModError::msg(format!(
                "module {}: resource/effect provider registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .resource_effect_provider_builder
            .register_effect(owner, definition, Arc::new(provider))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: effect provider registration: {e}",
                    self.module
                ))
            })
    }

    /// Declare a durable schedule family as a bounded wake source for one
    /// exact authoritative action owned by this module.
    ///
    /// Discovery enumerates the engine-owned typed family directly, so a
    /// provider cannot hide an unbounded process-local queue or index. The
    /// adapter supplies read-only domain addressing and guards after store
    /// preflight; the kernel re-evaluates and advances the cursor in the same
    /// authoritative bundle as the registered action's domain effects.
    pub fn schedule_source<A>(
        &mut self,
        id: crate::mechanics::CanonicalId,
        action: crate::mechanics::DefinitionRef,
        schedules: crate::action_state::StateFamilyHandle<
            crate::durable_schedule::ScheduleId,
            crate::durable_schedule::DurableSchedule,
        >,
        adapter: A,
    ) -> Result<(), ModError>
    where
        A: crate::schedule_source::ScheduleActionAdapter + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.schedule_sources.is_some() {
            return Err(ModError::msg(format!(
                "module {}: schedule-source registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .schedule_source_builder
            .register(owner, id, action, schedules, Arc::new(adapter))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: schedule source registration: {e}",
                    self.module
                ))
            })
    }

    /// Declare this module's authoritative delayed-decision families and the
    /// exact action used to durably record each provider lease.
    ///
    /// One source is permitted per owner. The kernel scans these families
    /// directly and seeds the lease post-image into the registered action's
    /// transaction; the action provider cannot start async work or choose a
    /// different record.
    pub fn decision_source<A>(
        &mut self,
        lease_action: crate::mechanics::DefinitionRef,
        families: crate::decision::DecisionCaptureFamilies,
        wake_adapter: crate::mechanics::CanonicalId,
        lease_duration: i64,
        fallback_adapter: A,
    ) -> Result<(), ModError>
    where
        A: crate::decision_source::DecisionFallbackAdapter + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.decision_sources.is_some() {
            return Err(ModError::msg(format!(
                "module {}: decision-source registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .decision_source_builder
            .register(
                owner,
                lease_action,
                families,
                wake_adapter,
                lease_duration,
                std::sync::Arc::new(fallback_adapter),
            )
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: decision source registration: {e}",
                    self.module
                ))
            })
    }

    /// Register the immutable-world evaluator for one revision kind owned by
    /// this module. Owner identity is stamped; no dotted-id inference occurs.
    pub fn revision_validator<V>(
        &mut self,
        kind: crate::action::RevisionKind,
        validator: V,
    ) -> Result<(), ModError>
    where
        V: crate::action::RevisionValidator + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.revision_validators.is_some() {
            return Err(ModError::msg(format!(
                "module {}: revision validator registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .revision_validator_builder
            .register(owner, kind, Arc::new(validator))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: revision validator registration: {e}",
                    self.module
                ))
            })
    }

    /// Register this module's final evaluator for one policy-owned authority
    /// guard category. Definition guards remain centrally exact-validated.
    pub fn authority_guard_validator<V>(
        &mut self,
        kind: crate::action::AuthorityGuardKind,
        validator: V,
    ) -> Result<(), ModError>
    where
        V: crate::action::AuthorityGuardValidator + 'static,
    {
        self.require_mechanics_registration_authority()?;
        if self.reg.authority_guard_validators.is_some() {
            return Err(ModError::msg(format!(
                "module {}: authority validator registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .authority_guard_validator_builder
            .register(owner, kind, Arc::new(validator))
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: authority validator registration: {e}",
                    self.module
                ))
            })
    }

    /// Register one directional cross-owner interaction bridge. K1 records
    /// only exact ownership/resolution; executable preparation arrives in K2.
    pub fn interaction_bridge(
        &mut self,
        descriptor: crate::mechanics::BridgeDescriptor,
    ) -> Result<(), ModError> {
        self.require_mechanics_registration_authority()?;
        if self.reg.bridges.is_some() {
            return Err(ModError::msg(format!(
                "module {}: bridge registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .bridge_builder
            .register(owner, descriptor)
            .map_err(|e| ModError::msg(format!("module {}: bridge registration: {e}", self.module)))
    }

    /// Register a pure, contiguous exact-definition migration edge. It is
    /// available for boot dry-runs only and never enters the eager namespace
    /// migration path below.
    pub fn definition_migration(
        &mut self,
        descriptor: crate::mechanics::DefinitionMigrationDescriptor,
    ) -> Result<(), ModError> {
        self.require_mechanics_registration_authority()?;
        if self.reg.definition_migrations.is_some() {
            return Err(ModError::msg(format!(
                "module {}: definition migration registry is already frozen",
                self.module
            )));
        }
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        self.reg
            .definition_migration_builder
            .register(owner, descriptor)
            .map_err(|e| {
                ModError::msg(format!(
                    "module {}: definition migration registration: {e}",
                    self.module
                ))
            })
    }

    /// Register a command. Identical to core registration except the entry
    /// is tagged with this module's identity (`ctx.module` in its handler;
    /// used by attr-namespace enforcement).
    ///
    /// A composed module must first call [`Registrar::classify_commands`].
    /// This fail-closed step makes adding a command without acknowledging its
    /// persistence lane a composition error rather than source-scanning debt.
    pub fn command(
        &mut self,
        spec: CommandSpec,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), ModError> {
        let compatibility = self.command_compatibility.ok_or_else(|| {
            ModError::msg(format!(
                "module {}: classify commands before registering {:?}",
                self.module, spec.name
            ))
        })?;
        self.reg
            .register_tagged(spec, handler, Some(self.module.clone()), compatibility)?;
        Ok(())
    }

    /// Set the compatibility class applied to subsequently registered
    /// commands.  A module may call this again between registration groups;
    /// every command stores the value captured at its own registration.
    pub fn classify_commands(&mut self, compatibility: crate::cmd::CommandCompatibility) {
        self.command_compatibility = Some(compatibility);
    }

    /// Append an alias-stage hook (the comsys seam; runs in registration
    /// order during dispatch).
    pub fn alias_stage(&mut self, stage: Arc<dyn AliasStage>) -> Result<(), ModError> {
        let compatibility = self.command_compatibility.ok_or_else(|| {
            ModError::msg(format!(
                "module {}: classify commands before registering an alias stage",
                self.module
            ))
        })?;
        self.reg
            .add_tagged_alias_stage(stage, Some(self.module.clone()), compatibility);
        Ok(())
    }

    /// Append a post-command-table [`CommandFallback`] (object
    /// `$`-commands — the mush-script seam; runs after built-ins/exits,
    /// before `Huh?`, in registration order).
    pub fn command_fallback(&mut self, fallback: Arc<dyn CommandFallback>) -> Result<(), ModError> {
        let compatibility = self.command_compatibility.ok_or_else(|| {
            ModError::msg(format!(
                "module {}: classify commands before registering a command fallback",
                self.module
            ))
        })?;
        self.reg
            .add_tagged_command_fallback(fallback, Some(self.module.clone()), compatibility);
        Ok(())
    }

    /// Append a pre-dispatch [`CommandGate`] (the onboarding gate — runs
    /// before the alias stage and `home`/exits/commands, in registration
    /// order, so it can gate channel broadcast and movement too). A module
    /// registers one to restrict what an actor may run based on live world
    /// state (e.g. an unapproved character held to the chargen command set).
    pub fn command_gate(&mut self, gate: Arc<dyn CommandGate>) {
        self.reg.add_command_gate(gate);
    }

    /// Register a typed behavior (design §4.2) owned by this module — the
    /// declaration bound to objects via `@behavior/add`. The id must be unique
    /// across modules; a collision is a [`ModError::Conflict`]. `spec.owner` is
    /// overwritten with this module's identity.
    pub fn behavior(&mut self, mut spec: crate::behavior::BehaviorSpec) -> Result<(), ModError> {
        spec.owner = self.module.clone();
        let module = self.module.clone();
        self.reg.register_behavior(spec).map_err(|e| {
            use crate::behavior::BehaviorRegError as E;
            match e {
                E::Owned { id, owner } => ModError::Conflict {
                    module: module.clone(),
                    what: "behavior",
                    name: SmolStr::new(id),
                    owner,
                },
                E::VerbCollision { id, verb, owner } => ModError::Conflict {
                    module: module.clone(),
                    what: "behavior command",
                    name: SmolStr::new(format!("{id}:{verb}")),
                    owner,
                },
                // Not a "registered by" conflict — report the error's own words.
                other => ModError::msg(format!("module {module}: {other}")),
            }
        })
    }

    /// Register a pre-destroy [`DestroyGate`](crate::behavior::DestroyGate)
    /// (design D8): a structural-dependency veto consulted at the command layer
    /// before `@destroy` mutates anything. `World` itself stays observer-free.
    pub fn destroy_gate(&mut self, gate: Arc<dyn crate::behavior::DestroyGate>) {
        self.reg.add_destroy_gate(gate);
    }

    /// Register a [`DetachGate`](crate::behavior::DetachGate): a veto consulted
    /// by `@behavior/remove` before a binding is detached, so a module can
    /// refuse stripping a behavior its records still depend on (a live domain's
    /// `sim.domain`). Keeps the documented `@behavior/remove` escape hatch from
    /// orphaning live structural state.
    pub fn detach_gate(&mut self, gate: Arc<dyn crate::behavior::DetachGate>) {
        self.reg.add_detach_gate(gate);
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
    /// construction and reachable via [`Ctx::state`] / [`Ctx::try_state_mut`].
    /// Two **different** modules declaring the same slot type is a boot
    /// error (checked by `Engine::with_modules`); a module re-declaring its
    /// own slot keeps the last value.
    ///
    /// Declaring the slot also records this module as its **owner**
    /// (`World::module_state_owner`), by module name and not by crate, so a
    /// crate registering two modules is two owners — and the enforced rule
    /// is: **only the owner may mutate the slot through `Ctx`.** A foreign
    /// module's [`Ctx::try_state_mut`] gets `Err(`[`StateSlotDenied`]`)`,
    /// its [`Ctx::state_mut`] gets `None`, and nothing is written; the
    /// refusal is logged once per (module, slot). Reading
    /// ([`Ctx::state`]) stays shared, a slot nobody declared stays open to
    /// all, and a core command (`module: None`) is a wildcard.
    ///
    /// The engine accessor `World::module_state_mut` carries no module
    /// identity and is not checked; `mush-contract-tests`'
    /// `state-slot-owners.toml` is what pins the cross-owner reaches that
    /// take it.
    pub fn state<T: Any + Send + Clone>(&mut self, init: T) {
        self.reg.tables_mut().state_inits.push(StateInit {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
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

    /// Publish this module's bounded lifecycle/incident summary to staff
    /// diagnostics. A module may register exactly one reporter.
    pub fn recovery_reporter(
        &mut self,
        reporter: Arc<dyn RecoveryReporter>,
    ) -> Result<(), ModError> {
        self.require_mechanics_registration_authority()?;
        let owner = crate::mechanics::OwnerId::new(self.module.as_str()).map_err(|e| {
            ModError::msg(format!(
                "module {} has no canonical mechanics owner id: {e}",
                self.module
            ))
        })?;
        if self
            .reg
            .tables_mut()
            .recovery_reporters
            .contains_key(&owner)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "recovery reporter",
                name: self.module.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg
            .tables_mut()
            .recovery_reporters
            .insert(owner, reporter);
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

    /// Declare that module `writer` may persist into namespace `ns`
    /// (WS03-A1) — the one way a cross-namespace module write is allowed to
    /// land.
    ///
    /// A module's [`GameModule::name`] is its persistence namespace, so
    /// [`Ctx::persist`](crate::cmd::Ctx::persist) refuses an op whose `ns` is
    /// not the writing module's own unless a row here says otherwise. The
    /// pair is what is declared, not a module or a namespace on its own: the
    /// jobs module's public API writes `jobs` documents under the CALLER's
    /// identity, which is a fact about that caller, not a licence for every
    /// module.
    ///
    /// **Who may declare a pair**: either module it names — the writer, or
    /// the namespace's owner. Both know why the write is legitimate; nobody
    /// else does. A ruleset therefore declares the pair its job payload
    /// handler produces (`jobs -> wod`: the handler runs under the jobs
    /// module's identity and writes the ruleset's namespace), and a producing
    /// module declares the pair its own call into the jobs API produces
    /// (`housing -> jobs`).
    ///
    /// # Errors
    /// A namespace equal to the writer (never a cross-namespace write); a
    /// pair naming neither this module as writer nor as namespace owner; or a
    /// pair already declared — first declaration wins, and a second is a
    /// conflict, exactly as it is for a migration namespace.
    pub fn persist_delegation(
        &mut self,
        writer: &str,
        ns: &str,
        reason: &'static str,
    ) -> Result<(), ModError> {
        if writer == ns {
            return Err(ModError::msg(format!(
                "module {}: {writer:?} -> {ns:?} is a module writing its own namespace, \
                 which needs no delegation",
                self.module
            )));
        }
        if writer != self.module && ns != self.module {
            return Err(ModError::msg(format!(
                "module {}: may not declare the delegation {writer:?} -> {ns:?} — a pair is \
                 declared by the writing module or by the namespace's owner, and this \
                 is neither",
                self.module
            )));
        }
        let (writer, ns) = (SmolStr::new(writer), SmolStr::new(ns));
        if let Some(existing) = self
            .reg
            .persist_delegations()
            .iter()
            .find(|d| d.writer == writer && d.ns == ns)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "persist delegation",
                name: SmolStr::new(format!("{writer} -> {ns}")),
                owner: existing.declared_by.clone(),
            });
        }
        let declared_by = self.module.clone();
        self.reg
            .tables_mut()
            .persist_delegations
            .push(PersistDelegation {
                writer,
                ns,
                declared_by,
                reason,
            });
        Ok(())
    }

    /// Register the game's single weather provider (the reserved
    /// `WeatherQuery` seam — see [`WeatherQuery`]). Exactly one module may
    /// provide weather; a second registration is a conflict.
    /// S6: register an interface-ability provider under `id` (doc 14 §4.6).
    /// One provider per id across the composition.
    ///
    /// # Errors
    ///
    /// [`ModError::Conflict`] when `id` is taken.
    pub fn interface_provider(
        &mut self,
        id: &str,
        provider: Arc<dyn InterfaceAbilityProvider>,
    ) -> Result<(), ModError> {
        let key = SmolStr::new(id);
        if let Some((owner, _)) = self.reg.tables_mut().interface_providers.get(&key) {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "interface provider",
                name: key,
                owner: owner.clone(),
            });
        }
        self.reg
            .tables_mut()
            .interface_providers
            .insert(key, (self.module.clone(), provider));
        Ok(())
    }

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

    /// Register this module's
    /// [`RequestCompletion`](crate::request::RequestCompletion) handler — the
    /// re-entry point for server-initiated requests it opens with
    /// [`Ctx::open_request`](crate::cmd::Ctx::open_request) (interface plan
    /// I10). A module registers at most one; a second is a conflict.
    /// Register this module as the one that speaks relayed lines onto its
    /// channels (programme D4). Exactly one module may.
    pub fn relay_speaker(&mut self, handler: Arc<dyn RelaySpeaker>) -> Result<(), ModError> {
        let tables = self.reg.tables_mut();
        if let Some((owner, _)) = &tables.relay_speaker {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "relay speaker",
                name: SmolStr::new_static("relay"),
                owner: owner.clone(),
            });
        }
        tables.relay_speaker = Some((self.module.clone(), handler));
        Ok(())
    }

    /// Register the handler for Discord component interactions whose
    /// `custom_id` starts with `prefix` (programme D3, 03 §7a.4). Prefixes are
    /// owner namespaces — `oncall:` — and the gateway keeps `d:` for itself;
    /// a prefix may be claimed once.
    pub fn discord_interaction(
        &mut self,
        prefix: &str,
        handler: Arc<dyn DiscordInteractionHandler>,
    ) -> Result<(), ModError> {
        let prefix = prefix.trim();
        if prefix.is_empty() || prefix.starts_with("d:") {
            return Err(ModError::msg(format!(
                "module {}: a Discord interaction prefix must be non-empty and outside `d:` \
                 (got {prefix:?})",
                self.module
            )));
        }
        let tables = self.reg.tables_mut();
        if let Some((_, owner, _)) = tables
            .discord_interactions
            .iter()
            .find(|(p, _, _)| p.as_str() == prefix)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "discord interaction prefix",
                name: SmolStr::new(prefix),
                owner: owner.clone(),
            });
        }
        tables
            .discord_interactions
            .push((SmolStr::new(prefix), self.module.clone(), handler));
        Ok(())
    }

    pub fn request_completion(
        &mut self,
        handler: Arc<dyn crate::request::RequestCompletion>,
    ) -> Result<(), ModError> {
        if self
            .reg
            .tables_mut()
            .request_completions
            .contains_key(&self.module)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "request completion",
                name: self.module.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg
            .tables_mut()
            .request_completions
            .insert(self.module.clone(), handler);
        Ok(())
    }
    /// Register this module's [`StoreReadCompletion`] handler — the re-entry
    /// point for its deferred store reads ([`Ctx::read_store`], H1). A module
    /// registers at most one; a second registration is a conflict. A module
    /// that never registers one may still call `read_store`, but the
    /// completion is then dropped with a log — so register first.
    pub fn store_read_completion(
        &mut self,
        handler: Arc<dyn StoreReadCompletion>,
    ) -> Result<(), ModError> {
        if self
            .reg
            .tables_mut()
            .store_read_completions
            .contains_key(&self.module)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "store read completion",
                name: self.module.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg
            .tables_mut()
            .store_read_completions
            .insert(self.module.clone(), handler);
        Ok(())
    }

    /// Register this module's one decision capture sink (H2a): the slot the
    /// driving loop arms with a built [`crate::decision::DecisionCapturePlan`]
    /// and the module's capture action provider drains inside its own
    /// `prepare_authoritative`. One per module; a second registration is a
    /// conflict.
    pub fn decision_capture_sink(
        &mut self,
        capture_action: crate::mechanics::DefinitionRef,
        sink: crate::decision::DecisionCaptureSink,
    ) -> Result<(), ModError> {
        if self
            .reg
            .tables_mut()
            .decision_capture_sinks
            .contains_key(&self.module)
        {
            return Err(ModError::Conflict {
                module: self.module.clone(),
                what: "decision capture sink",
                name: self.module.clone(),
                owner: self.module.clone(),
            });
        }
        self.reg
            .tables_mut()
            .decision_capture_sinks
            .insert(self.module.clone(), (capture_action, sink));
        Ok(())
    }
}

// ----------------------------------------------------------------------
// Firing helpers (used by dispatch, movement, engine)
// ----------------------------------------------------------------------

/// Consult every `MoveAttempt` hook; the first veto wins. `exit` is `None`
/// for `@teleport`.
///
/// `lens` is the caller's typed-state reach: a dispatched command hands over
/// its live store ([`Ctx::check_move_attempt`] does exactly that), and a
/// caller with no store names the reason. There is deliberately no store-less
/// spelling of this name.
pub fn check_move_attempt(
    reg: &Registry,
    w: &World,
    actor: ObjId,
    exit: Option<ObjId>,
    dest: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Result<(), RichText> {
    for h in reg.hooks(HookPoint::MoveAttempt) {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        h.hook.move_attempt(&ctx, actor, exit, dest)?;
    }
    Ok(())
}

/// Consult every `CarryAttempt` hook for `mover` moving `obj` into `dest`
/// (`get`/`drop`/`give`); the first veto wins. `lens` is the caller's
/// typed-state reach — see [`check_move_attempt`].
pub fn check_carry_attempt(
    reg: &Registry,
    w: &World,
    mover: ObjId,
    obj: ObjId,
    dest: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Result<(), RichText> {
    for h in reg.hooks(HookPoint::CarryAttempt) {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        h.hook.carry_attempt(&ctx, mover, obj, dest)?;
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
///
/// `lens` is how far this call can see into engine-owned typed state. There is
/// exactly one name — a caller with the live store passes
/// [`TypedStateLens::Live`](crate::TypedStateLens::Live), a caller without one
/// names the reason. The store-less spelling used to be the *shorter* name,
/// which made "no typed state" the accidental default and let a hook that
/// reads a typed family answer the wrong perception verdict.
#[must_use]
pub fn perceives_via(
    hooks: &[HookReg],
    w: &World,
    observer: ObjId,
    subject: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> bool {
    for h in hooks {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        if h.hook.perceives(&ctx, observer, subject) == Perceive::Conceal {
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
///
/// `lens` is the caller's typed-state reach — see [`perceives_via`].
#[must_use]
pub fn can_perceive(
    reg: &Registry,
    w: &World,
    observer: ObjId,
    subject: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> bool {
    perceives_via(reg.hooks(HookPoint::Perceive), w, observer, subject, lens)
}

/// Whether `subject` is under an active concealment effect according to ANY
/// registered `Perceive`-point hook (first-true-wins). The ruleset-agnostic
/// **notability** query — distinct from [`can_perceive`], which is the
/// per-observer projection: this asks "is this subject hidden at all?", not
/// "can THIS watcher see them?". With no hook registered (or none claiming
/// concealment) it is `false`, so it is additive and free — behavior identical
/// to a game with no concealment ruleset.
#[must_use]
pub fn is_concealed(
    reg: &Registry,
    w: &World,
    subject: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> bool {
    reg.hooks(HookPoint::Perceive).iter().any(|h| {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        h.hook.is_concealed(&ctx, subject)
    })
}

/// Collect every `LookRendered` hook's extra lines for a room render.
#[must_use]
pub fn look_rendered_lines(
    reg: &Registry,
    w: &World,
    looker: ObjId,
    room: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Vec<RichText> {
    let mut lines = Vec::new();
    for h in reg.hooks(HookPoint::LookRendered) {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        lines.extend(h.hook.look_rendered(&ctx, looker, room));
    }
    lines
}

/// Maximum keys in one `RoomView.ambient` map (interface plan I1 §7).
pub const MAX_AMBIENT_KEYS: usize = 8;
/// Maximum serialized bytes of one `RoomView.ambient` map (I1 §7).
pub const MAX_AMBIENT_BYTES: usize = 2048;

/// The key core itself fills in `RoomView.ambient` when no module has: the
/// IC clock as `{ unix, hour, minute, season, time_of_day }`.
pub const AMBIENT_IC_TIME_KEY: &str = "ic_time";

/// Collect every `RoomAmbient` hook's contributions for a room render, then
/// core's own `ic_time`, into one bounded map (first writer wins per key; a
/// contribution past [`MAX_AMBIENT_KEYS`] or [`MAX_AMBIENT_BYTES`] is dropped
/// with a warning rather than truncated).
#[must_use]
pub fn room_ambient(
    reg: &Registry,
    w: &World,
    looker: ObjId,
    room: ObjId,
    lens: crate::action_state::TypedStateLens<'_>,
) -> std::collections::BTreeMap<SmolStr, serde_json::Value> {
    let mut map = std::collections::BTreeMap::new();
    let mut bytes = 0usize;
    let mut offer = |key: SmolStr, value: serde_json::Value, module: &str| {
        if map.contains_key(&key) {
            tracing::warn!(module, %key, "room ambient: key already contributed; first writer kept");
            return;
        }
        if map.len() >= MAX_AMBIENT_KEYS {
            tracing::warn!(module, %key, "room ambient: key cap reached; contribution dropped");
            return;
        }
        // Key plus value plus the JSON punctuation around them; an estimate
        // that only ever over-counts.
        let cost = key.len() + 4 + serde_json::to_vec(&value).map_or(0, |v| v.len());
        if bytes + cost > MAX_AMBIENT_BYTES {
            tracing::warn!(module, %key, cost, "room ambient: byte cap reached; contribution dropped");
            return;
        }
        bytes += cost;
        map.insert(key, value);
    };
    for h in reg.hooks(HookPoint::RoomAmbient) {
        let ctx = HookReadContext::for_module(w, lens, &h.module);
        for (key, value) in h.hook.room_ambient(&ctx, looker, room) {
            offer(key, value, &h.module);
        }
    }
    let clock = w.game_clock();
    let now = w.now();
    offer(
        SmolStr::new_static(AMBIENT_IC_TIME_KEY),
        serde_json::json!({
            "unix": clock.ic_unix(now),
            "hour": clock.ic_hour(now),
            "minute": clock.ic_minute(now),
            "season": clock.season(now).to_string(),
            "time_of_day": clock.time_of_day(now).to_string(),
        }),
        "core",
    );
    map
}

/// Invoke every observer subscribed to `ev`'s kind, post-mutation, each
/// with a [`Ctx`] carrying its module identity.
pub fn emit_world_event(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    ev: &WorldEvent,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let observers = reg.observers_matching(ev);
    let actor = ev.subject();
    for (module, obs) in observers {
        let typed_state =
            crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
        let mut ctx = Ctx {
            w: &mut *w,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(module),
            typed_state,
        };
        obs.on_event(&mut ctx, ev);
    }
}

/// Route a [`WorldCommand::ModuleAsyncDone`](crate::engine::WorldCommand::ModuleAsyncDone)
/// to `module`'s registered [`AsyncCompletion`] handler, in the world task,
/// with a full [`Ctx`] carrying the module identity. A missing handler is
/// logged and dropped (the completion arrived for a module that registered
/// none — a programming error, never a crash).
#[allow(clippy::too_many_arguments)]
pub fn apply_async_completion(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    module: SmolStr,
    actor: ObjId,
    correlation: u64,
    payload: &str,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let Some(handler) = reg.async_completion(&module) else {
        tracing::warn!(%module, correlation, "async completion for unregistered module dropped");
        return;
    };
    let typed_state =
        crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
    let mut ctx = Ctx {
        w,
        actor,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
        typed_state,
    };
    handler.on_complete(&mut ctx, correlation, payload);
}

/// A Discord component interaction forwarded by the gateway (programme D3,
/// 03 §7a.4). The bridge authenticated the Discord user (the Gateway asserts
/// the id) and resolved the linked account; the handler trusts both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscordInteraction {
    /// The linked account's display name, if the user is linked.
    pub account: Option<String>,
    /// The character the bridge chose for the account (followed, else relay,
    /// else first), if any — also the `Ctx` actor when present.
    pub actor: Option<ObjId>,
    /// The full routing key (`oncall:ack:17`).
    pub custom_id: String,
    /// A select's picked values or a modal's field values, in order.
    pub values: Vec<String>,
}

/// The handler a module registers for a `custom_id` prefix
/// ([`Registrar::discord_interaction`]). It runs on the world loop with a
/// [`Ctx`] whose actor is the interaction's character (or `GOD` when the
/// account has none), may mutate, and returns the bounded, viewer-safe text
/// Discord shows the clicker privately — or a refusal reason.
pub trait DiscordInteractionHandler: Send + Sync {
    /// Handle one interaction.
    ///
    /// # Errors
    ///
    /// A bounded, viewer-safe reason the interaction was refused.
    fn handle(&self, ctx: &mut Ctx<'_>, interaction: &DiscordInteraction)
    -> Result<String, String>;
}

/// Why a relayed line was not spoken (programme D4). Every arm is a bounded,
/// viewer-safe reason the bridge may show the Discord author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayRefusal {
    /// No module speaks relayed lines.
    NoSpeaker,
    /// No channel of that name.
    NoSuchChannel,
    /// The channel is not flagged for the relay.
    NotRelay,
    /// The character is not on the channel.
    NotMember,
    /// The character has the channel gagged.
    Gagged,
    /// The channel's speak lock refuses the character.
    SpeakLock,
    /// Nothing to say.
    Empty,
}

impl std::fmt::Display for RelayRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RelayRefusal::NoSpeaker => "the game has no channel relay",
            RelayRefusal::NoSuchChannel => "no such channel",
            RelayRefusal::NotRelay => "that channel is not relayed",
            RelayRefusal::NotMember => "your character is not on that channel",
            RelayRefusal::Gagged => "your character has that channel gagged",
            RelayRefusal::SpeakLock => "your character cannot speak on that channel",
            RelayRefusal::Empty => "nothing to say",
        })
    }
}

/// The handler a module registers to speak relayed lines
/// ([`Registrar::relay_speaker`], programme D4): the comsys module, which runs
/// membership, gag and speak-lock exactly as its alias stage does and marks
/// the line's origin so it is never relayed back out.
pub trait RelaySpeaker: Send + Sync {
    /// Speak `text` as `ctx.actor` on the channel named `channel`.
    ///
    /// # Errors
    ///
    /// The refusal the Discord author may be shown.
    fn relay_speak(
        &self,
        ctx: &mut Ctx<'_>,
        channel: &str,
        kind: mush_proto::NarrativeKind,
        text: &str,
    ) -> Result<(), RelayRefusal>;
}

/// Run the registered relay speaker as `actor` (programme D4).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_relay_speak(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    actor: ObjId,
    channel: &str,
    kind: mush_proto::NarrativeKind,
    text: &str,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Result<(), RelayRefusal> {
    let Some((module, handler)) = reg.relay_speaker() else {
        return Err(RelayRefusal::NoSpeaker);
    };
    let typed_state =
        crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
    let mut ctx = Ctx {
        w,
        actor,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
        typed_state,
    };
    handler.relay_speak(&mut ctx, channel, kind, text)
}

/// Dispatch a forwarded Discord interaction to the module handler with the
/// longest matching prefix. `None` when no prefix matches.
pub(crate) fn apply_discord_interaction(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    interaction: &DiscordInteraction,
    lens: crate::action_state::TypedStateLens<'_>,
) -> Option<Result<String, String>> {
    let (module, handler) = reg.discord_interaction_handler(&interaction.custom_id)?;
    let actor = interaction.actor.unwrap_or(crate::GOD);
    let typed_state =
        crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
    let mut ctx = Ctx {
        w,
        actor,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
        typed_state,
    };
    Some(handler.handle(&mut ctx, interaction))
}

/// Route a resolved server-initiated request (interface plan I10) to
/// `module`'s registered [`RequestCompletion`](crate::request::RequestCompletion),
/// in the world task, with a full [`Ctx`] whose `actor` is the subject. A
/// missing handler is logged and dropped. The handler re-validates against
/// current world state (treaty ruling 3).
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_request_completion(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    module: SmolStr,
    subject: ObjId,
    payload: &str,
    outcome: crate::request::RequestOutcome,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let Some(handler) = reg.request_completion(&module) else {
        tracing::warn!(%module, "request completion for unregistered module dropped");
        return;
    };
    let typed_state =
        crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
    let mut ctx = Ctx {
        w,
        actor: subject,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
        typed_state,
    };
    handler.on_respond(&mut ctx, payload, outcome);
}

/// Route a [`WorldCommand::StoreReadDone`](crate::engine::WorldCommand::StoreReadDone)
/// to `module`'s registered [`StoreReadCompletion`] handler, in the world
/// task, with a full [`Ctx`] carrying the module identity (H1). A missing
/// handler is logged and dropped — the same contract as
/// [`apply_async_completion`].
#[allow(clippy::too_many_arguments)]
pub fn apply_store_read_completion(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    module: SmolStr,
    actor: ObjId,
    correlation: u64,
    payload: &str,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let Some(handler) = reg.store_read_completion(&module) else {
        tracing::warn!(
            %module,
            correlation,
            "store read completion for a module with no registered handler dropped"
        );
        return;
    };
    let result = StoreReadResult::from_payload(payload);
    let typed_state =
        crate::action_state::ModuleActionState::for_module(lens, Some(module.as_str()));
    let mut ctx = Ctx {
        w,
        actor,
        switches: SwitchSet::default(),
        args: ParsedArgs::default(),
        out,
        reg,
        module: Some(module),
        typed_state,
    };
    handler.on_read(&mut ctx, correlation, result);
}

/// Which session transition [`fire_conn_hook`] is dispatching (16b cut K.2, K4).
///
/// Cut K.2 needed two facts the kernel had but never published: the **onset** of
/// a linkdead gap, and a **revival** inside the grace. The dispatcher's old
/// `connected: bool` could not carry either, so the flag became this word, and
/// `from_linkdead` at the engine's own connect site is what picks
/// [`Revived`](ConnTransition::Revived) over
/// [`Connected`](ConnTransition::Connected).
///
/// Revival deliberately rides [`HookPoint::Connected`]'s **registrations** on a
/// second trait method rather than a `HookPoint` of its own: nine modules hook
/// `Connected` today (mail, comsys, the economy inbox, housing, weather, the
/// game sheet, the arena and two test rigs), and firing their `connected` on
/// every reconnect would hand each of them a behaviour change this cut has no
/// business making. [`EventHook::revived`] defaults to nothing, so they are
/// byte-identical and the one module that cares overrides it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConnTransition {
    /// A session became live (the world-side connect transition).
    Connected,
    /// A session became live again inside the linkdead grace.
    Revived,
    /// The last connection dropped uncleanly; the grace is running.
    Linkdead,
    /// The session ended (a clean quit, or the grace expiring).
    Disconnected,
}

impl ConnTransition {
    /// The point whose registrations this transition dispatches over.
    #[must_use]
    pub const fn point(self) -> HookPoint {
        match self {
            // A revival is a connect to everyone who registered for one.
            ConnTransition::Connected | ConnTransition::Revived => HookPoint::Connected,
            ConnTransition::Linkdead => HookPoint::Linkdead,
            ConnTransition::Disconnected => HookPoint::Disconnected,
        }
    }
}

/// Fire the session-transition hooks for an actor (see [`ConnTransition`]).
pub fn fire_conn_hook(
    reg: &Registry,
    w: &mut World,
    out: &mut Outbox,
    actor: ObjId,
    transition: ConnTransition,
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let hooks: Vec<HookReg> = reg.hooks(transition.point()).to_vec();
    for h in hooks {
        let typed_state =
            crate::action_state::ModuleActionState::for_module(lens, Some(h.module.as_str()));
        let mut ctx = Ctx {
            w: &mut *w,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(h.module.clone()),
            typed_state,
        };
        match transition {
            ConnTransition::Connected => h.hook.connected(&mut ctx, actor),
            ConnTransition::Revived => h.hook.revived(&mut ctx, actor),
            ConnTransition::Linkdead => h.hook.linkdead(&mut ctx, actor),
            ConnTransition::Disconnected => h.hook.disconnected(&mut ctx, actor),
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
    lens: crate::action_state::TypedStateLens<'_>,
) {
    let point = if started {
        HookPoint::SceneStarted
    } else {
        HookPoint::SceneEnded
    };
    let hooks: Vec<HookReg> = reg.hooks(point).to_vec();
    for h in hooks {
        let typed_state =
            crate::action_state::ModuleActionState::for_module(lens, Some(h.module.as_str()));
        let mut ctx = Ctx {
            w: &mut *w,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut *out,
            reg,
            module: Some(h.module.clone()),
            typed_state,
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
    /// Build the immutable view used when a command invokes a registered
    /// hook directly (for example, while deliberately skipping its own
    /// module's veto). The hook is stamped with its registering owner and
    /// receives the same live typed-state snapshot as ordinary dispatch.
    #[must_use]
    pub fn hook_read_context<'a>(&'a self, module: &str) -> HookReadContext<'a> {
        HookReadContext::for_module(self.w, self.typed_state.lens(), module)
    }

    /// Consult the registered movement veto chain with this command's live
    /// typed-state snapshot.
    pub fn check_move_attempt(
        &self,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        check_move_attempt(self.reg, self.w, actor, exit, dest, self.typed_state.lens())
    }

    /// The gate a narrative move passes — the exit's `Basic` lock, then the
    /// movement veto chain — for a move made cell by cell (S8.1): the atlas
    /// asks it at a room boundary with the portal's exit, or with none where
    /// two footprints simply touch.
    ///
    /// # Errors
    ///
    /// The refusal, as the mover reads it.
    pub fn authorize_traversal(
        &self,
        actor: ObjId,
        exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), crate::commands::movement::TraversalRefusal> {
        crate::commands::movement::authorize_traversal(
            self.reg,
            self.w,
            actor,
            exit,
            dest,
            self.typed_state.lens(),
        )
    }

    /// Consult the registered custody veto chain with this command's live
    /// typed-state snapshot.
    pub fn check_carry_attempt(
        &self,
        mover: ObjId,
        obj: ObjId,
        dest: ObjId,
    ) -> Result<(), RichText> {
        check_carry_attempt(self.reg, self.w, mover, obj, dest, self.typed_state.lens())
    }

    /// Resolve per-observer visibility with this command's live typed state.
    #[must_use]
    pub fn can_perceive(&self, observer: ObjId, subject: ObjId) -> bool {
        perceives_via(
            self.reg.hooks(HookPoint::Perceive),
            self.w,
            observer,
            subject,
            self.typed_state.lens(),
        )
    }

    /// Resolve ruleset-agnostic concealment with this command's live typed state.
    #[must_use]
    pub fn is_concealed(&self, subject: ObjId) -> bool {
        is_concealed(self.reg, self.w, subject, self.typed_state.lens())
    }

    /// Render a room through perception and look hooks using this command's
    /// live typed-state snapshot.
    pub fn show_room(&mut self, looker: ObjId, room: ObjId) {
        crate::commands::show_room(
            self.reg,
            self.w,
            looker,
            room,
            self.out,
            self.typed_state.lens(),
        );
    }

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
    /// **Trust model** (WS03-A1): `op.ns` is verified against `ctx.module`,
    /// and an undeclared cross-namespace write is REFUSED — the op is
    /// dropped, nothing is queued, and nothing lands.
    ///
    /// A module's [`GameModule::name`] is its persistence namespace, so
    /// `op.ns != ctx.module` is one module writing documents another module
    /// owns. That is still *sanctioned delegation* where it is sanctioned —
    /// the jobs module's public API takes `ctx` and writes `jobs` documents
    /// under the CALLER's identity — but the delegation must now be
    /// **declared**, per `(writer, namespace)` pair and with its reason, by
    /// one of the two modules the pair names
    /// ([`Registrar::persist_delegation`]). Compiled-in modules are trusted
    /// code; what is not trusted is a delegation set nobody wrote down, which
    /// is what this replaces.
    ///
    /// A refusal never panics — this runs on the single world task — and is
    /// loud in two places at once: one `error!` per `(module, namespace)`
    /// rather than one per command, and a row on the world for
    /// [`World::refused_namespace_writes`](crate::world::World::refused_namespace_writes).
    /// It has to be: dropping a write is silent at the call site, since
    /// `persist` returns nothing to ignore.
    ///
    /// **What the check cannot see.** `persist` is not the only spelling of a
    /// module write. [`Deferred::Persist`](crate::engine::Deferred) is a public
    /// variant and [`Outbox::defer`](crate::Outbox::defer) is public, so the
    /// crate-local `push_op` helper most modules actually write through queues
    /// the op without ever passing here: it is not compared, was not in the
    /// census, and cannot be refused. Nothing crosses a namespace that way
    /// today — every one of those helpers stamps its own crate's namespace
    /// constant by construction, which is what makes that path safe rather
    /// than checked — but a write that MEANS to cross a namespace must be
    /// spelled `ctx.persist` to be governed at all. Fencing the sibling path
    /// is a follow-up step, not something this one invented: a pinned scan
    /// over non-`mush-core` source for a `Deferred::Persist` whose namespace
    /// is not the crate's own constant.
    ///
    /// A core command (`module: None`) is a wildcard, exactly as it is for
    /// state slots. The engine's own namespace-crossing helpers — the scene
    /// API, which writes the scene's OWNING namespace whoever calls it, and
    /// the world timeline, which writes an engine-owned namespace no module
    /// may be named after — take [`Ctx::persist_engine_owned`] instead: the
    /// engine mediates those, so they are not a module reaching across.
    pub fn persist(&mut self, op: ModuleOp) {
        // This is on every module write path, so the fast path is one
        // SmolStr equality and nothing else; the delegation table is only
        // ever consulted on a mismatch.
        if self.module.as_ref().is_some_and(|module| *module != op.ns)
            && !self.may_persist_into(&op.ns)
        {
            return;
        }
        self.out.defer(Deferred::Persist(op));
    }

    /// Queue a persistence op whose namespace the ENGINE, not this module,
    /// decides: the scene API's writes into a scene's owning namespace and
    /// the world timeline's writes into its reserved one.
    ///
    /// Not a way around [`Ctx::persist`]'s namespace check — it is the
    /// spelling for the writes that check would misread, and it lives in
    /// `mush-core` alone (`pub(crate)`), so a module cannot reach it.
    pub(crate) fn persist_engine_owned(&mut self, op: ModuleOp) {
        self.out.defer(Deferred::Persist(op));
    }

    /// Whether this module may write a namespace that is not its own, and
    /// record the refusal when it may not (WS03-A1). Cold: only ever reached
    /// on a mismatch.
    #[cold]
    fn may_persist_into(&mut self, ns: &SmolStr) -> bool {
        // A core command carries no module identity and is a wildcard.
        let Some(module) = self.module.clone() else {
            return true;
        };
        if self.reg.persist_delegation(&module, ns).is_some() {
            return true;
        }
        let owner = self.reg.namespace_owner(ns).cloned();
        self.w.record_refused_namespace_write(&module, ns, owner);
        false
    }

    /// Open a server-initiated request to `subject` (interface plan I10,
    /// D-I-21): the client is asked and answers by id, or the deadline fires
    /// this module's default; either way the engine routes the outcome to this
    /// module's registered
    /// [`RequestCompletion`](crate::request::RequestCompletion). Choice-only —
    /// the server owns the effect. Ephemeral / RAM-only (D-I-22): a request
    /// outstanding at a restart is lost. A no-op when this context carries no
    /// module identity. Options past
    /// [`MAX_REQUEST_OPTIONS`](mush_proto::MAX_REQUEST_OPTIONS) are dropped; a
    /// negative timeout clamps to zero (fires on the next scheduler pass).
    pub fn open_request(
        &mut self,
        subject: ObjId,
        kind: mush_proto::RequestKind,
        prompt: mush_proto::RichText,
        options: Vec<mush_proto::RequestOption>,
        timeout_ms: i64,
        payload: impl Into<String>,
    ) {
        let Some(module) = self.module.clone() else {
            tracing::warn!("open_request with no module identity; dropped");
            return;
        };
        // Bound the prompt defensively (verify): a module is trusted code, but
        // a runaway prompt must never reach the wire. Over budget -> a short
        // plain replacement.
        let prompt = if prompt.to_plain_text().len() > mush_proto::MAX_REQUEST_PROMPT_BYTES {
            mush_proto::RichText::tag("system", "(prompt too long)")
        } else {
            prompt
        };
        self.out
            .defer(Deferred::OpenRequest(crate::request::OpenRequest {
                subject,
                kind,
                prompt,
                options: options
                    .into_iter()
                    .take(mush_proto::MAX_REQUEST_OPTIONS)
                    .collect(),
                timeout_ms: timeout_ms.max(0),
                module,
                payload: payload.into(),
            }));
    }

    // ---- E7c2: B1 — the `Authoritative` ingress adapter seam ----
    /// H2a (D-H-6): ask the driving loop to capture one durable decision for
    /// `draft.actor` under this module's decision source. A selector, never an
    /// offer: the loop projects the actor's offers, witnesses them, signs the
    /// prompt and admits the record ([`crate::Engine::build_decision_capture`]),
    /// arms this module's registered capture sink, and submits the module's
    /// capture action through the one authoritative ingress. One capture per
    /// step; a second voids both.
    ///
    /// # Errors
    /// [`DecisionCaptureRequestError::NoModule`] outside a module handler;
    /// [`DecisionCaptureRequestError::Owner`] when the module's name is not a
    /// canonical mechanics owner.
    pub fn capture_decision(
        &mut self,
        draft: crate::decision::DecisionCaptureDraft,
    ) -> Result<(), DecisionCaptureRequestError> {
        let module = self
            .module
            .as_ref()
            .ok_or(DecisionCaptureRequestError::NoModule)?;
        let owner = crate::mechanics::OwnerId::new(module.as_str())
            .map_err(|error| DecisionCaptureRequestError::Owner(error.to_string()))?;
        self.out.defer(Deferred::DecisionCapture {
            owner,
            draft: Box::new(draft),
        });
        Ok(())
    }

    /// Submit one registered authoritative action for this command's actor.
    ///
    /// This is the `Ctx`-level action submit 08j §5 (iii) and 08i §2 row 5
    /// named as missing, and it is the **only** way a
    /// [`CommandCompatibility::Authoritative`](crate::cmd::CommandCompatibility::Authoritative)
    /// handler reaches `ActionCoordinator::prepare_and_stage_resolved`.
    ///
    /// It is a **selector, not an offer**. The handler names a registered
    /// action definition, the subject it expects, its bounded targets and its
    /// parameters; the driving loop then regenerates *this actor's own*
    /// projected legal actions, resolves the matching offer, and runs the same
    /// five steps as the `WorldMsg::AuthoritativeAction` arm — one code path,
    /// not two spellings. A handler therefore cannot construct an opaque
    /// `ActionOfferId`, cannot skip projection, and cannot target an action the
    /// actor has no offer for.
    ///
    /// **Contract for the handler.** Perform no world or module mutation of
    /// your own and emit no success line: the whole mechanic belongs to the
    /// registered provider, and the actor's confirmation is released only after
    /// the durable receipt. **Calling this twice in one command is a refusal**
    /// — the engine stages nothing at all *for that actor* and tells them so,
    /// leaving every other actor's queued submission alone (E7c2 fold C4) —
    /// because a command that fans out could commit one half and lose the
    /// other.
    pub fn submit_authoritative(
        &mut self,
        action: crate::mechanics::DefinitionRef,
        subject: crate::action::PerceivedRef,
        targets: Vec<crate::action::PerceivedRef>,
        parameters: serde_json::Value,
        key: crate::action::IdempotencyKeyText,
    ) {
        self.out.defer(Deferred::AuthoritativeSubmit(Box::new(
            crate::engine::AuthoritativeSubmit {
                actor: self.actor,
                action,
                subject,
                targets,
                parameters,
                key,
            },
        )));
    }

    // ---- H1: the module store-read seam ----
    /// Queue a bounded, off-loop read of documents from **this module's own**
    /// namespace, completing later through the module's registered
    /// [`StoreReadCompletion`] with the returned correlation id.
    ///
    /// The namespace is `ctx.module`, never a parameter, so no module can read
    /// another's documents; `keys` are namespace-relative (a ledger page is
    /// `chunk_key(log_id, n)` for a handful of `n`). At most
    /// [`MAX_STORE_READ_KEYS`] keys; the driving loop fetches each with one
    /// `module_json_get` — never a listing — and caps the returned bytes at
    /// [`MAX_STORE_READ_BYTES`]. The world loop never waits.
    ///
    /// Like every deferred item, a read queued by a handler that then returns
    /// `Err` is discarded with the rest of its deferred work.
    pub fn read_store(&mut self, keys: Vec<SmolStr>) -> Result<u64, StoreReadError> {
        let Some(module) = self.module.clone() else {
            return Err(StoreReadError::NoModule);
        };
        if keys.is_empty() {
            return Err(StoreReadError::NoKeys);
        }
        if keys.len() > MAX_STORE_READ_KEYS {
            return Err(StoreReadError::TooManyKeys {
                requested: keys.len(),
                max: MAX_STORE_READ_KEYS,
            });
        }
        for key in &keys {
            if key.is_empty() {
                return Err(StoreReadError::EmptyKey);
            }
            if key.len() > MAX_MODULE_KEY_BYTES {
                return Err(StoreReadError::KeyTooLong {
                    bytes: key.len(),
                    max: MAX_MODULE_KEY_BYTES,
                });
            }
        }
        let correlation = self.w.next_correlation_id();
        self.out
            .defer(Deferred::Effect(crate::engine::Effect::StoreRead {
                module,
                actor: self.actor,
                correlation,
                keys,
            }));
        Ok(correlation)
    }

    /// Queue a notice to one actor on the unified notification surface
    /// (interface plan I11) — see [`Outbox::notice_to_actor`](crate::output::Outbox::notice_to_actor).
    /// Stamps the world clock for the wire's `ts` (`World::now()` is seconds;
    /// the wire wants milliseconds).
    pub fn notice(
        &mut self,
        actor: ObjId,
        severity: mush_proto::NoticeSeverity,
        category: &'static str,
        text: impl Into<String>,
        action: Option<(&str, &str)>,
    ) {
        let now_ms = self.w.now().saturating_mul(1000);
        self.out
            .notice_to_actor(actor, now_ms, severity, category, text, action);
    }

    /// [`notice`](Self::notice) at `Info` severity.
    pub fn notice_info(
        &mut self,
        actor: ObjId,
        category: &'static str,
        text: impl Into<String>,
        action: Option<(&str, &str)>,
    ) {
        self.notice(
            actor,
            mush_proto::NoticeSeverity::Info,
            category,
            text,
            action,
        );
    }

    /// [`notice`](Self::notice) at `Warn` severity.
    pub fn notice_warn(
        &mut self,
        actor: ObjId,
        category: &'static str,
        text: impl Into<String>,
        action: Option<(&str, &str)>,
    ) {
        self.notice(
            actor,
            mush_proto::NoticeSeverity::Warn,
            category,
            text,
            action,
        );
    }

    /// [`notice`](Self::notice) at `Urgent` severity.
    pub fn notice_urgent(
        &mut self,
        actor: ObjId,
        category: &'static str,
        text: impl Into<String>,
        action: Option<(&str, &str)>,
    ) {
        self.notice(
            actor,
            mush_proto::NoticeSeverity::Urgent,
            category,
            text,
            action,
        );
    }

    /// Mark the acting player as the preferred recipient for a durable-write
    /// failure diagnostic. The server already receipt-gates every dirty step's
    /// commit-bound output and all external effects; this marker preserves the
    /// stronger, command-specific warning for flows such as mail send and XP
    /// approval.
    pub fn gate_on_durable_ack(&mut self) {
        self.out.defer(Deferred::DurableGate {
            actor: self.actor,
            strict: false,
        });
    }

    /// Like [`Ctx::gate_on_durable_ack`], but **strict** (for money — E2 R-R1): on a
    /// dropped durable write the step's confirmation is SUPPRESSED entirely, not
    /// delivered with a "may not survive" warning. Invariant 9 — a posted economic
    /// confirmation must never outrun the durable ack (the RAM effect is frozen and
    /// reconciled at boot; the actor is told the command did NOT confirm). Use it
    /// from any handler that emits a money-movement confirmation.
    pub fn gate_on_durable_ack_strict(&mut self) {
        self.out.defer(Deferred::DurableGate {
            actor: self.actor,
            strict: true,
        });
    }

    /// Read this world's state slot for `T` (declared via
    /// [`Registrar::state`]).
    ///
    /// Reading is **shared on purpose** and is not owner-checked: a module
    /// reading another module's slot is a projection, not a write, and the
    /// tree does it deliberately (`mush-propagation` reads `mush-tactics`'
    /// contact ledger to decide what propagates). Mutation is the half that
    /// belongs to one module — see [`Ctx::try_state_mut`].
    #[must_use]
    pub fn state<T: Any + Send>(&self) -> Option<&T> {
        self.w.module_state::<T>()
    }

    /// Mutably access this world's state slot for `T`, **refusing** a slot
    /// another module declared.
    ///
    /// `Err(`[`StateSlotDenied`]`)` names the requesting module and the
    /// owner, and nothing is written. `Ok(None)` is the slot simply not being
    /// installed — an optional module did not compose — which is a normal
    /// answer and not a refusal. Nothing here ever panics: this runs on the
    /// single world task.
    ///
    /// A slot with no recorded owner (seeded through
    /// [`World::module_state_insert`](crate::world::World::module_state_insert)
    /// rather than declared) stays open to all, and a core command —
    /// `module: None` — is a wildcard.
    ///
    /// This is the checked spelling, and it is the one to reach for: the
    /// refusal is only as strong as the surface it sits on, and `Ctx::w` is a
    /// `pub` field, so `ctx.w.module_state_mut::<T>()` skips the check
    /// entirely. `mush-contract-tests`' `state-slot-owners.toml` is what
    /// holds that route.
    pub fn try_state_mut<T: Any + Send>(&mut self) -> Result<Option<&mut T>, StateSlotDenied> {
        let slot = std::any::type_name::<T>();
        match self
            .w
            .check_module_state_write(self.module.as_ref(), TypeId::of::<T>(), slot)
        {
            Ok(()) => Ok(self.w.module_state_mut::<T>()),
            Err(owner) => Err(StateSlotDenied {
                requester: self.module.clone().unwrap_or_default(),
                owner,
                slot,
            }),
        }
    }

    /// [`Ctx::try_state_mut`] for handlers that treat a missing slot and a
    /// refused one alike.
    ///
    /// `None` means the write did not happen — because the slot is absent, or
    /// because it belongs to another module. A module reaching its OWN slot
    /// only ever sees the first, so the familiar
    /// `if let Some(s) = ctx.state_mut::<T>()` keeps its meaning; a handler
    /// that wants to tell the two apart (or to surface the refusal) calls
    /// `try_state_mut` and reads the [`StateSlotDenied`]. Either way the
    /// refusal is logged once per (module, slot) and recorded on the world
    /// for
    /// [`World::refused_state_writes`](crate::world::World::refused_state_writes),
    /// so a regression is visible rather than a write that quietly stops
    /// landing.
    pub fn state_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        self.try_state_mut::<T>().ok().flatten()
    }

    /// Read one record from an engine-owned typed family registered by this
    /// context's module. This is a read-only capability: authoritative
    /// changes still require an [`ActionTxn`](crate::ActionTxn).
    pub fn action_state<K, V>(
        &self,
        handle: &crate::action_state::StateFamilyHandle<K, V>,
        key: &K,
    ) -> Result<Option<&V>, crate::action_state::ModuleActionStateReadError>
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.typed_state.get(handle, key)
    }

    /// Scan one bounded canonical page of a typed family registered by this
    /// context's module.
    pub fn scan_action_state<K, V>(
        &self,
        handle: &crate::action_state::StateFamilyHandle<K, V>,
        after: Option<&str>,
        limit: usize,
    ) -> Result<
        crate::action_state::StateFamilyPage<'_, V>,
        crate::action_state::ModuleActionStateReadError,
    >
    where
        K: Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        self.typed_state.scan_family(handle, after, limit)
    }

    /// Emit a world event to subscribed observers (post-mutation). Command
    /// handlers that mutate the world through paths not covered by the
    /// core helpers call this themselves.
    pub fn emit_world_event(&mut self, ev: &WorldEvent) {
        emit_world_event(self.reg, self.w, self.out, ev, self.typed_state.lens());
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
        fire_scene_hook(
            self.reg,
            self.w,
            self.out,
            id,
            true,
            self.typed_state.lens(),
        );
        id
    }

    /// End a scene: fires `SceneEnded` hooks (scene still queryable), then
    /// removes it and its per-object ruleset state, queuing a `Del`
    /// persistence op for the latter (in the scene's **owning** namespace,
    /// regardless of who calls this). Scene ticks stop on their next due
    /// time. Returns whether the scene existed.
    pub fn end_scene(&mut self, id: SceneId) -> bool {
        if self.w.scene(id).is_none() {
            return false;
        }
        fire_scene_hook(
            self.reg,
            self.w,
            self.out,
            id,
            false,
            self.typed_state.lens(),
        );
        let Some(scene) = self.w.scene_remove(id) else {
            return false;
        };
        let ns = scene.ns.clone();
        // The scene's OWNING namespace, whoever ends it: engine-mediated, so
        // it takes the spelling WS03-A1's check does not read as a module
        // reaching across.
        self.persist_engine_owned(ModuleOp {
            ns,
            op: ModuleWrite::Del {
                key: scene.ruleset_state_key.clone(),
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
        self.persist_engine_owned(ModuleOp {
            ns,
            op: ModuleWrite::JsonSet {
                key,
                value: snapshot,
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

    /// A store-less lens for the assertions below: these tests pin the answer
    /// a caller gets when it has no typed state at all, which is exactly the
    /// answer the old store-less twins handed out by default.
    const DETACHED: crate::action_state::TypedStateLens<'static> =
        crate::action_state::TypedStateLens::Detached(
            crate::action_state::DetachedTypedState::Test,
        );

    /// With no `Perceive` hook registered, the notability query defaults to
    /// `false` — additive and free (a game with no concealment ruleset).
    #[test]
    fn is_concealed_defaults_false_with_no_hook() {
        let reg = Registry::new();
        let w = World::new_minimal();
        assert!(!is_concealed(&reg, &w, ObjId(1), DETACHED));
        // And `can_perceive` remains a free pass on the same empty registry.
        assert!(can_perceive(&reg, &w, ObjId(1), ObjId(2), DETACHED));
    }

    /// A registered hook whose `is_concealed` returns true makes the firing
    /// helper report the subject concealed (first-true-wins).
    #[test]
    fn is_concealed_true_when_a_hook_claims_it() {
        struct AlwaysConcealed;
        impl EventHook for AlwaysConcealed {
            fn is_concealed(&self, _ctx: &HookReadContext<'_>, _subject: ObjId) -> bool {
                true
            }
        }
        let mut reg = Registry::new();
        Registrar::new(&mut reg, "test").hook(HookPoint::Perceive, Arc::new(AlwaysConcealed));
        let w = World::new_minimal();
        assert!(is_concealed(&reg, &w, ObjId(1), DETACHED));
    }
}
