//! The [`World`]: every object in RAM, dense by id, with maintained inverse
//! indexes and a dirty set for the (later) persistence layer.
//!
//! `World` is plain synchronous data — no I/O, no async, no locks. One task
//! will own it outright at runtime; here it is fully unit-testable. All
//! mutation goes through `World` methods, which maintain **all** invariants:
//!
//! - `location` ↔ `contents`/`exits` stay inverse of each other,
//! - actors are findable in the player index by name and alias,
//! - `zone` ↔ zone-membership index stay in sync,
//! - every touched object lands in the dirty set,
//! - containment stays acyclic.
//!
//! Time: the world never calls `SystemTime`. The driving loop feeds wall
//! time in via [`World::set_now`]; `created`/`modified` stamps use it.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use mush_proto::PromptSpec;
use smol_str::SmolStr;

use crate::attr::{Attr, AttrFlags, UncasedName};
use crate::behavior::{BehaviorBinding, BehaviorId};
use crate::clock::{GameClock, Season, TimeOfDay};
use crate::flags::Flags;
use crate::module::{InputContext, InputStacks, ModuleStateMap, Scene, SceneId, StateInit};
use crate::obj::{GOD, LIMBO, Obj, ObjId, ObjKind};

/// Maximum `@parent` hops for attribute inheritance lookups.
pub const MAX_PARENT_DEPTH: usize = 10;

/// Maximum hops up the `location` chain when finding an object's containing
/// room (design 06 §4.3, "bounded location chain").
pub const MAX_CONTAINMENT_DEPTH: usize = 8;

/// A published "actor operates subject" link (design 06 §4.4) — how a pilot in
/// a cockpit gets the vehicle as a command subject, or a rigger a remote drone.
/// Transient and rebuildable: the publishing `owner` module holds the
/// authoritative assignment and republishes on boot; core never persists it.
/// Keyed on `(actor, subject, owner)`; within an actor's list, higher
/// `priority` wins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSubjectLink {
    /// The operating actor.
    pub actor: ObjId,
    /// The operated object.
    pub subject: ObjId,
    /// The relation (`"pilot"`, `"gunner"`, `"remote"`, …) — descriptive.
    pub relation: SmolStr,
    /// The module that published it (responsible for republish/removal).
    pub owner: SmolStr,
    /// Higher wins among an actor's links.
    pub priority: u16,
    /// The control channels this link GRANTS the actor over the subject. The
    /// publisher unions an actor's seats before publishing; the resolver unions
    /// across multiple owners' links. Transient (republished at boot), so it is
    /// never serialized.
    pub channels: std::collections::BTreeSet<super::behavior::ChannelId>,
}

/// Errors from [`World`] mutators.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorldError {
    /// The referenced object does not exist (never created, or swept).
    #[error("no such object: {0}")]
    NoSuchObj(ObjId),
    /// The move would put an object inside itself (directly or transitively).
    #[error("moving {obj} into {dest} would create a containment cycle")]
    Cycle {
        /// Object being moved.
        obj: ObjId,
        /// Attempted destination.
        dest: ObjId,
    },
    /// An actor name or alias is already taken by another actor.
    #[error("name {0:?} is already in use")]
    NameInUse(String),
    /// `link_exit` on a non-exit.
    #[error("{0} is not an exit")]
    NotAnExit(ObjId),
    /// `#0` and `#1` may not be destroyed.
    #[error("{0} may not be destroyed")]
    Protected(ObjId),
    /// The parent assignment would create a `@parent` loop.
    #[error("parent loop: {obj} cannot have {parent} as a parent")]
    ParentCycle {
        /// Object being reparented.
        obj: ObjId,
        /// Attempted parent.
        parent: ObjId,
    },
    /// [`World::from_objects`] was handed two objects with the same id.
    #[error("duplicate object id {0} in load set")]
    DuplicateId(ObjId),
    /// [`World::from_objects`] was handed a `next_id` that does not exceed
    /// every loaded id (ids are never recycled, so the allocator must
    /// resume strictly above them).
    #[error("next_id {next_id} must be greater than loaded id {id}")]
    BadNextId {
        /// The `next_id` passed in.
        next_id: u32,
        /// A loaded object id at or above it.
        id: ObjId,
    },
}

/// Transient per-actor session timing, fed by the engine as connections
/// come and go and read by `WHO`. Never persisted, never dirty-tracked.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SessionStat {
    /// Unix seconds when the actor's first live connection attached.
    pub connect_time: i64,
    /// Unix seconds of the actor's most recent input line.
    pub last_input: i64,
}

/// The entire game state, in RAM, dense by [`ObjId`].
///
/// Ids are never recycled: a destroyed-and-swept object leaves a permanent
/// `None` tombstone in `objs`, so `objs[id]` is always either *the* object
/// with that id or garbage — never a different object.
#[derive(Clone, Debug, Default)]
pub struct World {
    /// Dense object storage; index == id. `None` = tombstone (or never used).
    objs: Vec<Option<Obj>>,
    /// Next id to allocate (monotonic).
    next_id: u32,
    /// Uncased actor name **and** alias → id, players and NPCs only.
    player_index: HashMap<UncasedName, ObjId>,
    /// Zone Master Object → members, maintained by [`World::set_zone`].
    zone_members: HashMap<ObjId, BTreeSet<ObjId>>,
    /// Objects mutated since the last [`World::take_dirty`].
    dirty: BTreeSet<ObjId>,
    /// Actors currently marked connected (transient session state for WHO
    /// and routing; never persisted, never dirty-tracked).
    connected: BTreeSet<ObjId>,
    /// Objects locked into an **exclusive** scene (a turn-based timestop): a
    /// transient index a scene driver maintains so core flows can ask "is this
    /// object in a turn-based scene?" without depending on the driver's module
    /// (the driver owns the rich record; core owns only this membership bit).
    /// The movement gate and the NPC wake loop consult it; the driver rebuilds
    /// it on boot from its persisted scene records. Never persisted here.
    exclusive_scene: BTreeSet<ObjId>,
    /// Published command-subject links (design 06 §4.4): per actor, the objects
    /// it operates (a pilot's mech, a rigger's drone), highest priority first.
    /// A transient, rebuildable index: the owning module holds the authoritative
    /// assignment and republishes on boot; never persisted here. Scrubbed by
    /// [`World::destroy`] in both roles.
    command_subject_links: HashMap<ObjId, Vec<CommandSubjectLink>>,
    /// Per-actor session timing (transient, like `connected`).
    session_stats: HashMap<ObjId, SessionStat>,
    /// Live transport connections per actor, mirrored in by the engine
    /// (transient). Differs from `connected` during link-dead grace: the
    /// actor stays *connected* (WHO, room presence) while holding **zero**
    /// live connections — nothing sent to them is actually delivered.
    live_conns: HashMap<ObjId, u32>,
    /// Monotonic correlation-id source for async re-entry (treaty ruling 3).
    next_correlation: u64,
    /// Wall-clock unix seconds, fed by the driver via [`World::set_now`].
    now: i64,
    /// The IC game clock (IC:OOC ratio + calendar phase). Boot configuration,
    /// not persisted and not dirty-tracked; the driver installs it from
    /// `mushd.toml [game]` via [`World::set_game_clock`] and it derives IC
    /// time purely from [`World::now`]. `Default` is 1:1 with OOC time.
    game_clock: GameClock,
    /// Active scenes (transient world state; module data persists via
    /// module ops).
    scenes: BTreeMap<SceneId, Scene>,
    /// Monotonic scene-id source (per boot).
    next_scene_id: u64,
    /// Per-`(scene, obj)` ruleset state (the `RulesetStateStore`).
    scene_state: BTreeMap<(SceneId, ObjId), serde_json::Value>,
    /// Per-scene map blobs (opaque to core).
    scene_maps: BTreeMap<SceneId, serde_json::Value>,
    /// Typed module state slots (anymap; in-RAM working state).
    module_state: ModuleStateMap,
    /// Per-actor input-context stacks (transient parser state — empty on
    /// clone).
    input_stacks: InputStacks,
}

impl World {
    /// A minimal bootable world: room `#0` "Limbo" and player `#1` "God"
    /// (WIZARD flag, homed and located in Limbo). Both start dirty.
    #[must_use]
    pub fn new_minimal() -> World {
        let mut limbo = Obj::new(LIMBO, ObjKind::Room, "Limbo", GOD, 0);
        let mut god = Obj::new(GOD, ObjKind::Player, "God", GOD, 0);
        god.location = Some(LIMBO);
        god.home = Some(LIMBO);
        god.flags = Flags::WIZARD;
        limbo.contents.push(GOD);

        let mut w = World {
            objs: vec![Some(limbo), Some(god)],
            next_id: 2,
            ..World::default()
        };
        w.player_index.insert(UncasedName::from("God"), GOD);
        w.dirty.insert(LIMBO);
        w.dirty.insert(GOD);
        w
    }

    /// Rebuild a `World` from persisted objects — the hydration path the
    /// M2 store uses at boot (`obj:{id}` docs out of Redis, `next_id` from
    /// its counter).
    ///
    /// The **inverse indexes are derived, not trusted**: each object's
    /// stored `contents`/`exits` lists are discarded and rebuilt from the
    /// `location` fields (in ascending-id order), the player index from
    /// actor names/aliases, and zone membership from `zone` fields — so a
    /// loaded world always satisfies every `World` invariant.
    ///
    /// Validation (all fail the load):
    /// - duplicate ids ([`WorldError::DuplicateId`]);
    /// - an id at or above `next_id` ([`WorldError::BadNextId`]);
    /// - a `location` or `zone` referencing a missing object
    ///   ([`WorldError::NoSuchObj`]);
    /// - two live actors claiming the same name or alias
    ///   ([`WorldError::NameInUse`]).
    ///
    /// The loaded world starts with an **empty dirty set** (loading is not
    /// a mutation) and `now == 0`; the driver calls [`World::set_now`]
    /// before dispatching commands. Gaps below `next_id` become tombstones,
    /// exactly as if those ids had been destroyed and swept.
    pub fn from_objects(
        objects: impl IntoIterator<Item = Obj>,
        next_id: u32,
    ) -> Result<World, WorldError> {
        let mut slots: Vec<Option<Obj>> = Vec::new();
        slots.resize_with(next_id as usize, || None);
        for mut o in objects {
            if o.id.0 >= next_id {
                return Err(WorldError::BadNextId { next_id, id: o.id });
            }
            let idx = o.id.0 as usize;
            if slots[idx].is_some() {
                return Err(WorldError::DuplicateId(o.id));
            }
            // Inverse indexes are rebuilt below.
            o.contents.clear();
            o.exits.clear();
            slots[idx] = Some(o);
        }

        let mut w = World {
            objs: slots,
            next_id,
            ..World::default()
        };
        let ids: Vec<ObjId> = w.iter().map(|o| o.id).collect();
        for id in ids {
            let (kind, location, zone, going) = {
                let o = w.get(id).expect("just inserted");
                (o.kind, o.location, o.zone, o.flags.contains(Flags::GOING))
            };
            if let Some(loc) = location {
                w.ensure(loc)?;
                if let Some(holder) = w.slot_mut(loc) {
                    if kind == ObjKind::Exit {
                        holder.exits.push(id);
                    } else {
                        holder.contents.push(id);
                    }
                }
            }
            if let Some(z) = zone {
                w.ensure(z)?;
                w.zone_members.entry(z).or_default().insert(id);
            }
            if kind.is_actor() && !going {
                let keys: Vec<UncasedName> = {
                    let o = w.get(id).expect("just inserted");
                    let mut keys = vec![UncasedName::from(o.name.as_str())];
                    keys.extend(o.aliases.iter().map(UncasedName::from));
                    keys
                };
                for k in keys {
                    if let Some(&other) = w.player_index.get(&k)
                        && other != id
                    {
                        return Err(WorldError::NameInUse(k.as_str().to_string()));
                    }
                    w.player_index.insert(k, id);
                }
            }
        }
        Ok(w)
    }

    // ------------------------------------------------------------------
    // Time
    // ------------------------------------------------------------------

    /// Update the world's notion of "now" (unix seconds). The driving loop
    /// calls this; core logic never reads the system clock itself.
    pub fn set_now(&mut self, unix_seconds: i64) {
        self.now = unix_seconds;
    }

    /// The last time fed in via [`World::set_now`].
    #[must_use]
    pub fn now(&self) -> i64 {
        self.now
    }

    // ------------------------------------------------------------------
    // IC game clock (read model for commands, modules, scripts)
    // ------------------------------------------------------------------

    /// Install the IC game clock (the driver calls this once at boot from
    /// `mushd.toml [game]`; `ic_ratio` is therefore read at boot — hot-reload
    /// comes later). Purely configuration: nothing is dirtied or persisted.
    pub fn set_game_clock(&mut self, clock: GameClock) {
        self.game_clock = clock;
    }

    /// The configured IC game clock.
    #[must_use]
    pub fn game_clock(&self) -> GameClock {
        self.game_clock
    }

    /// IC ("in-character") unix-style timestamp for the current wall time —
    /// the game clock applied to [`World::now`]. Modules and scripts read the
    /// IC clock through these accessors rather than touching
    /// [`World::now`] (which is OOC wall time).
    #[must_use]
    pub fn game_now_unix(&self) -> i64 {
        self.game_clock.ic_unix(self.now)
    }

    /// The IC season right now.
    #[must_use]
    pub fn season(&self) -> Season {
        self.game_clock.season(self.now)
    }

    /// The IC time-of-day bucket right now.
    #[must_use]
    pub fn time_of_day(&self) -> TimeOfDay {
        self.game_clock.time_of_day(self.now)
    }

    /// IC hour of day right now, `0..24`.
    #[must_use]
    pub fn ic_hour(&self) -> u32 {
        self.game_clock.ic_hour(self.now)
    }

    /// IC minute of the hour right now, `0..60`.
    #[must_use]
    pub fn ic_minute(&self) -> u32 {
        self.game_clock.ic_minute(self.now)
    }

    /// IC day of the current month, `1..=` its length.
    #[must_use]
    pub fn ic_day(&self) -> u32 {
        self.game_clock.ic_day(self.now)
    }

    /// IC day of the current year, `0..365`.
    #[must_use]
    pub fn ic_day_of_year(&self) -> u32 {
        self.game_clock.ic_day_of_year(self.now)
    }

    /// IC month right now, `1..=12`.
    #[must_use]
    pub fn ic_month(&self) -> u32 {
        self.game_clock.ic_month(self.now)
    }

    /// IC year right now.
    #[must_use]
    pub fn ic_year(&self) -> i64 {
        self.game_clock.ic_year(self.now)
    }

    /// The human-facing IC datetime string right now
    /// (`"August 28, 2075, 02:57"`).
    #[must_use]
    pub fn ic_datetime(&self) -> String {
        self.game_clock.ic_datetime(self.now)
    }

    /// The next id the allocator will hand out (monotonic, never recycled).
    /// The persistence layer stores this beside the objects so
    /// [`World::from_objects`] can resume allocation exactly where it
    /// stopped.
    #[must_use]
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// Look up an object. `None` for never-allocated ids and tombstones.
    #[must_use]
    pub fn get(&self, id: ObjId) -> Option<&Obj> {
        self.objs.get(id.0 as usize).and_then(Option::as_ref)
    }

    /// Mutable lookup. Marks the object dirty, since the caller is
    /// presumably about to change it. Prefer the typed mutators — they
    /// maintain indexes; raw `get_mut` is for fields with no index
    /// (markers, locks, flags you're only reading, ...).
    pub fn get_mut(&mut self, id: ObjId) -> Option<&mut Obj> {
        if self.get(id).is_some() {
            self.dirty.insert(id);
        }
        self.objs.get_mut(id.0 as usize).and_then(Option::as_mut)
    }

    /// Attach a typed behavior binding (see [`crate::behavior`]), overwriting any
    /// existing binding for the same id, and mark the object dirty. This is the
    /// raw mutator; policy checks (kind, conflicts, permission) live at the
    /// command layer, which has the [`Registry`](crate::cmd::Registry).
    pub fn attach_behavior(
        &mut self,
        id: ObjId,
        behavior: BehaviorId,
        binding: BehaviorBinding,
    ) -> Result<(), WorldError> {
        self.ensure(id)?;
        if let Some(o) = self.slot_mut(id) {
            o.behaviors.insert(behavior, binding);
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Remove a behavior binding, returning whether it was present. Marks the
    /// object dirty only when something was removed.
    pub fn detach_behavior(
        &mut self,
        id: ObjId,
        behavior: &BehaviorId,
    ) -> Result<bool, WorldError> {
        self.ensure(id)?;
        let removed = self
            .slot_mut(id)
            .is_some_and(|o| o.behaviors.remove(behavior).is_some());
        if removed {
            self.touch_modified(id);
        }
        Ok(removed)
    }

    /// Whether `id` carries `behavior`.
    #[must_use]
    pub fn has_behavior(&self, id: ObjId, behavior: &BehaviorId) -> bool {
        self.get(id)
            .is_some_and(|o| o.behaviors.contains_key(behavior))
    }

    /// Iterate all live objects in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Obj> {
        self.objs.iter().filter_map(Option::as_ref)
    }

    /// The contents list of an object (empty slice if the object is gone).
    #[must_use]
    pub fn contents_of(&self, id: ObjId) -> &[ObjId] {
        self.get(id).map_or(&[], |o| o.contents.as_slice())
    }

    /// The exits list of an object (empty slice if the object is gone).
    #[must_use]
    pub fn exits_of(&self, id: ObjId) -> &[ObjId] {
        self.get(id).map_or(&[], |o| o.exits.as_slice())
    }

    /// Resolve an actor (player/NPC) by name or alias, case-insensitively.
    #[must_use]
    pub fn resolve_player(&self, name: &str) -> Option<ObjId> {
        self.player_index.get(&UncasedName::from(name)).copied()
    }

    /// Members of a zone (empty iterator for non-zones).
    pub fn zone_members_of(&self, zone: ObjId) -> impl Iterator<Item = ObjId> + '_ {
        self.zone_members.get(&zone).into_iter().flatten().copied()
    }

    /// Attribute lookup with `@parent`-chain inheritance.
    ///
    /// The object's own attributes always win. Ancestors are walked up to
    /// [`MAX_PARENT_DEPTH`] hops (which also bounds accidental parent
    /// cycles); an ancestor's attribute flagged
    /// [`NO_INHERIT`](AttrFlags::NO_INHERIT) is skipped and the walk
    /// continues upward.
    #[must_use]
    pub fn attr(&self, obj: ObjId, name: &str) -> Option<&Attr> {
        let key = UncasedName::from(name);
        let mut cur = self.get(obj)?;
        if let Some(a) = cur.attrs.get_uncased(&key) {
            return Some(a);
        }
        for _ in 0..MAX_PARENT_DEPTH {
            cur = self.get(cur.parent?)?;
            if let Some(a) = cur.attrs.get_uncased(&key)
                && !a.flags.contains(AttrFlags::NO_INHERIT)
            {
                return Some(a);
            }
        }
        None
    }

    /// Drain and return the set of objects mutated since the last call.
    /// The persistence layer treats an id whose slot is `None` as a delete.
    pub fn take_dirty(&mut self) -> BTreeSet<ObjId> {
        std::mem::take(&mut self.dirty)
    }

    /// Whether authoritative object state has an unflushed legacy mutation.
    /// Used by the engine to keep prepared action-state submissions separate
    /// from the compatibility snapshot lane.
    #[must_use]
    pub(crate) fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    // ------------------------------------------------------------------
    // Exclusive-scene membership (the timestop index)
    // ------------------------------------------------------------------

    /// Whether `obj` is locked into an exclusive turn-based scene. Core flows
    /// consult this — the movement gate refuses to let a participant wander
    /// off, and the NPC wake loop suppresses free-running turns for a
    /// participant so it acts only on its scene turn. The scene driver owns the
    /// rich record and maintains this bit via
    /// [`World::set_exclusive_scene_member`] / [`World::clear_exclusive_scene_member`].
    #[must_use]
    pub fn in_exclusive_scene(&self, obj: ObjId) -> bool {
        self.exclusive_scene.contains(&obj)
    }

    /// All objects currently in an exclusive scene, in id order.
    pub fn exclusive_scene_members(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.exclusive_scene.iter().copied()
    }

    /// Mark `obj` a member of an exclusive scene (idempotent). The scene driver
    /// calls this when it freezes membership, and again on boot when it
    /// rehydrates active scenes. Transient — never persisted by core.
    pub fn set_exclusive_scene_member(&mut self, obj: ObjId) {
        self.exclusive_scene.insert(obj);
    }

    /// Clear `obj`'s exclusive-scene membership (idempotent). The driver calls
    /// this when the scene releases the object (ends, or drops a participant).
    pub fn clear_exclusive_scene_member(&mut self, obj: ObjId) {
        self.exclusive_scene.remove(&obj);
    }

    // ------------------------------------------------------------------
    // Command-subject links (design 06 §4.4; transient, rebuildable)
    // ------------------------------------------------------------------

    /// Publish (upsert) a command-subject link. Keyed on `(actor, subject,
    /// owner)`: republishing replaces the relation and priority. A no-op if
    /// either object does not exist. Never persisted — the owner republishes
    /// on boot from its own records.
    pub fn publish_command_subject(&mut self, link: CommandSubjectLink) {
        if self.get(link.actor).is_none() || self.get(link.subject).is_none() {
            return;
        }
        let list = self.command_subject_links.entry(link.actor).or_default();
        list.retain(|l| !(l.subject == link.subject && l.owner == link.owner));
        list.push(link);
        list.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.subject.cmp(&b.subject)));
    }

    /// Retract links published by `owner` — for one `actor`, or for every
    /// actor when `None`. Returns how many were removed.
    pub fn retract_command_subjects(&mut self, owner: &str, actor: Option<ObjId>) -> usize {
        let keys: Vec<ObjId> = match actor {
            Some(a) => vec![a],
            None => self.command_subject_links.keys().copied().collect(),
        };
        let mut removed = 0;
        for a in keys {
            if let Some(list) = self.command_subject_links.get_mut(&a) {
                let before = list.len();
                list.retain(|l| l.owner != owner);
                removed += before - list.len();
                if list.is_empty() {
                    self.command_subject_links.remove(&a);
                }
            }
        }
        removed
    }

    /// The links published for `actor`, highest priority first (empty when
    /// none).
    #[must_use]
    pub fn command_subjects_of(&self, actor: ObjId) -> &[CommandSubjectLink] {
        self.command_subject_links
            .get(&actor)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Drop every link in which `obj` is the actor or the subject. Called by
    /// [`World::destroy`] so a dead dbref never lingers as a command subject.
    fn remove_command_subject_links_for_obj(&mut self, obj: ObjId) {
        self.command_subject_links.remove(&obj);
        self.command_subject_links.retain(|_, list| {
            list.retain(|l| l.subject != obj);
            !list.is_empty()
        });
    }

    /// The room containing `id`: walk `location` upward, bounded by
    /// [`MAX_CONTAINMENT_DEPTH`], until a [`Room`](ObjKind::Room). A room's
    /// containing room is itself. `None` if the chain ends or the bound is hit.
    /// Follows only `location` — never `parent`, `zone`, or `home`.
    #[must_use]
    pub fn containing_room(&self, id: ObjId) -> Option<ObjId> {
        let mut cur = id;
        for _ in 0..=MAX_CONTAINMENT_DEPTH {
            let o = self.get(cur)?;
            if o.kind == ObjKind::Room {
                return Some(cur);
            }
            cur = o.location?;
        }
        None
    }

    /// Whether [`World::move_to`]`(obj, dest)` would succeed — exactly its
    /// validation (both exist; `dest` is neither `obj` nor transitively inside
    /// it) with no mutation. A caller that must not half-apply (a spatial
    /// transition projecting several objects at once) pre-validates every move
    /// with this, after which `move_to` cannot fail.
    pub fn can_move_to(&self, obj: ObjId, dest: ObjId) -> Result<(), WorldError> {
        self.ensure(obj)?;
        self.ensure(dest)?;
        if obj == dest {
            return Err(WorldError::Cycle { obj, dest });
        }
        let mut cur = Some(dest);
        let mut hops = 0usize;
        while let Some(c) = cur {
            if c == obj {
                return Err(WorldError::Cycle { obj, dest });
            }
            cur = self.get(c).and_then(|o| o.location);
            hops += 1;
            if hops > self.objs.len() {
                break; // pre-existing cycle above us; don't loop forever
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Connection state (transient session bookkeeping)
    // ------------------------------------------------------------------

    /// Mark an actor connected or disconnected. Connection state is
    /// transient session bookkeeping (WHO, routing); it is not persisted
    /// and does not dirty the object. Marking a missing object is a no-op.
    pub fn set_connected(&mut self, id: ObjId, on: bool) {
        if on {
            if self.get(id).is_some() {
                self.connected.insert(id);
            }
        } else {
            self.connected.remove(&id);
        }
    }

    /// Whether the actor is currently marked connected.
    #[must_use]
    pub fn is_connected(&self, id: ObjId) -> bool {
        self.connected.contains(&id)
    }

    /// Iterate currently connected actors in id order.
    pub fn connected(&self) -> impl Iterator<Item = ObjId> + '_ {
        self.connected.iter().copied()
    }

    /// Mirror an actor's live transport-connection count (engine
    /// bookkeeping — transient, never persisted). A link-dead actor is
    /// still [`is_connected`](World::is_connected) but has zero live
    /// connections.
    pub fn set_live_conns(&mut self, id: ObjId, n: u32) {
        if n == 0 {
            self.live_conns.remove(&id);
        } else if self.get(id).is_some() {
            self.live_conns.insert(id, n);
        }
    }

    /// Whether the actor has at least one live transport connection —
    /// i.e. output sent to them right now is actually delivered. Modules
    /// use this (not [`is_connected`](World::is_connected)) when advancing
    /// delivery cursors, so link-dead players never "read" what they never
    /// received.
    #[must_use]
    pub fn has_live_conns(&self, id: ObjId) -> bool {
        self.live_conns.get(&id).copied().unwrap_or(0) > 0
    }

    /// Record a session starting for `id` **now** (both `connect_time` and
    /// `last_input` set to [`World::now`]). Idempotent: an existing stat is
    /// kept, so extra connections attaching to the same actor never reset
    /// the connect time. No-op for missing objects.
    pub fn session_start(&mut self, id: ObjId) {
        if self.get(id).is_some() {
            let now = self.now;
            self.session_stats.entry(id).or_insert(SessionStat {
                connect_time: now,
                last_input: now,
            });
        }
    }

    /// Stamp `last_input` for `id` with [`World::now`]. No-op if no session
    /// is recorded (e.g. script/NPC origin).
    pub fn session_input(&mut self, id: ObjId) {
        let now = self.now;
        if let Some(s) = self.session_stats.get_mut(&id) {
            s.last_input = now;
        }
    }

    /// Forget the session timing for `id` (the session fully ended).
    pub fn session_end(&mut self, id: ObjId) {
        self.session_stats.remove(&id);
    }

    /// The session timing recorded for `id`, if any.
    #[must_use]
    pub fn session_stat(&self, id: ObjId) -> Option<SessionStat> {
        self.session_stats.get(&id).copied()
    }

    /// Allocate a fresh correlation id for async re-entry (treaty ruling 3:
    /// deferred work carries one out and its typed [`WorldCommand`]
    /// completion carries it back). Transient — restarts from 0 each boot,
    /// which is fine because in-flight work does not survive a restart.
    ///
    /// [`WorldCommand`]: crate::engine::WorldCommand
    pub fn next_correlation_id(&mut self) -> u64 {
        let id = self.next_correlation;
        self.next_correlation += 1;
        id
    }

    // ------------------------------------------------------------------
    // Mutators
    // ------------------------------------------------------------------

    /// Create a new object, allocate its id, and wire it into the world.
    ///
    /// - `Player`s own themselves (the `owner` argument is ignored for
    ///   them, per the classic model); everything else needs an existing
    ///   owner.
    /// - `location`, when given, must exist; exits are wired into its
    ///   `exits` list, everything else into `contents`.
    /// - Actors get `home` defaulted to their creation location, must have
    ///   a name not already claimed by another actor, and are indexed.
    pub fn create(
        &mut self,
        kind: ObjKind,
        name: &str,
        owner: ObjId,
        location: Option<ObjId>,
    ) -> Result<ObjId, WorldError> {
        if kind != ObjKind::Player {
            self.ensure(owner)?;
        }
        if let Some(loc) = location {
            self.ensure(loc)?;
        }
        if kind.is_actor() && self.player_index.contains_key(&UncasedName::from(name)) {
            return Err(WorldError::NameInUse(name.to_string()));
        }

        let id = ObjId(self.next_id);
        debug_assert_eq!(self.objs.len(), id.0 as usize, "objs must stay dense");
        let owner = if kind == ObjKind::Player { id } else { owner };
        let mut obj = Obj::new(id, kind, name, owner, self.now);
        obj.location = location;
        if kind.is_actor() {
            obj.home = location;
        }
        self.objs.push(Some(obj));
        self.next_id += 1;

        if let Some(loc) = location {
            if let Some(holder) = self.slot_mut(loc) {
                if kind == ObjKind::Exit {
                    holder.exits.push(id);
                } else {
                    holder.contents.push(id);
                }
            }
            self.touch_modified(loc);
        }
        if kind.is_actor() {
            self.reindex_actor(id);
        }
        self.dirty.insert(id);
        Ok(id)
    }

    /// Mark an object `GOING` and detach it from the live world:
    /// deindexed, dezoned, removed from its holder, its contents sent to
    /// their homes (or Limbo), its exits destroyed with it. The husk stays
    /// resolvable (flagged `GOING`) until [`World::sweep`] tombstones it.
    ///
    /// Returns every id newly marked `GOING` — the object plus any exits taken
    /// with it — so the command layer can emit one `ObjDestroyed` per husk (the
    /// module-cleanup backstop). An already-`GOING` object yields an empty vec.
    ///
    /// `#0` and `#1` are protected. This is observer-free and does **not**
    /// consult the pre-destroy veto ([`Registry::check_destroy`](crate::cmd::Registry::check_destroy));
    /// the veto lives at the command layer.
    pub fn destroy(&mut self, id: ObjId) -> Result<Vec<ObjId>, WorldError> {
        if id == GOD || id == LIMBO {
            return Err(WorldError::Protected(id));
        }
        let (going, kind, location, zone, contents, exits) = {
            let o = self.get(id).ok_or(WorldError::NoSuchObj(id))?;
            (
                o.flags.contains(Flags::GOING),
                o.kind,
                o.location,
                o.zone,
                o.contents.clone(),
                o.exits.clone(),
            )
        };
        if going {
            return Ok(Vec::new());
        }

        if let Some(o) = self.slot_mut(id) {
            o.flags.insert(Flags::GOING);
        }
        if kind.is_actor() {
            self.player_index.retain(|_, v| *v != id);
        }
        self.connected.remove(&id);
        self.live_conns.remove(&id);
        self.session_stats.remove(&id);
        self.input_stacks.clear(id);
        self.remove_command_subject_links_for_obj(id);
        for s in self.scenes.values_mut() {
            s.members.remove(&id);
        }
        // Drop the husk's per-scene ruleset state too, or the next scene
        // snapshot re-persists dead-object entries forever. Scene owners
        // that need richer cleanup (ending emptied scenes, stopping ticks)
        // subscribe `WorldEventKind::ObjDestroyed` — the dispatch layer
        // emits it right after this returns.
        self.scene_state.retain(|(_, obj), _| *obj != id);
        // Leave any zone it was in...
        if zone.is_some() {
            self.set_zone(id, None)?;
        }
        // ...and if it WAS a zone, release its members.
        if let Some(members) = self.zone_members.remove(&id) {
            for m in members {
                if let Some(o) = self.slot_mut(m) {
                    o.zone = None;
                }
                self.touch_modified(m);
            }
        }
        // Detach from its holder.
        if let Some(loc) = location {
            if let Some(holder) = self.slot_mut(loc) {
                holder.contents.retain(|&c| c != id);
                holder.exits.retain(|&c| c != id);
            }
            self.touch_modified(loc);
            if let Some(o) = self.slot_mut(id) {
                o.location = None;
            }
        }
        // Send contents home (or to Limbo).
        for c in contents {
            let home = self
                .get(c)
                .and_then(|o| o.home)
                .filter(|&h| h != id)
                .filter(|&h| {
                    self.get(h)
                        .is_some_and(|ho| !ho.flags.contains(Flags::GOING))
                });
            let dest = home.unwrap_or(LIMBO);
            if self.move_to(c, dest).is_err() {
                let _ = self.move_to(c, LIMBO);
            }
        }
        // A destroyed room takes its exits with it.
        let mut destroyed = vec![id];
        for e in exits {
            if let Ok(sub) = self.destroy(e) {
                destroyed.extend(sub);
            }
        }
        self.touch_modified(id);
        Ok(destroyed)
    }

    /// Remove every `GOING` object, leaving permanent tombstones (their ids
    /// are never reused). Returns the swept ids; each is also marked dirty
    /// so the persistence layer sees the deletion.
    pub fn sweep(&mut self) -> Vec<ObjId> {
        let going: Vec<ObjId> = self
            .iter()
            .filter(|o| o.flags.contains(Flags::GOING))
            .map(|o| o.id)
            .collect();
        for &id in &going {
            if let Some(slot) = self.objs.get_mut(id.0 as usize) {
                *slot = None;
            }
            self.dirty.insert(id);
        }
        going
    }

    /// Move an object into `dest`, maintaining both sides' lists.
    ///
    /// Rejects containment cycles by walking `dest`'s location chain: you
    /// cannot move a thing into itself or into anything it (transitively)
    /// contains. Exits land in `dest.exits` (relinking their source room);
    /// everything else lands in `dest.contents`.
    pub fn move_to(&mut self, obj: ObjId, dest: ObjId) -> Result<(), WorldError> {
        self.can_move_to(obj, dest)?;

        let (kind, old_loc) = {
            let o = self.get(obj).ok_or(WorldError::NoSuchObj(obj))?;
            (o.kind, o.location)
        };
        if old_loc == Some(dest) {
            return Ok(());
        }
        if let Some(old) = old_loc {
            if let Some(holder) = self.slot_mut(old) {
                holder.contents.retain(|&x| x != obj);
                holder.exits.retain(|&x| x != obj);
            }
            self.touch_modified(old);
        }
        if let Some(holder) = self.slot_mut(dest) {
            if kind == ObjKind::Exit {
                holder.exits.push(obj);
            } else {
                holder.contents.push(obj);
            }
        }
        self.touch_modified(dest);
        if let Some(o) = self.slot_mut(obj) {
            o.location = Some(dest);
        }
        self.touch_modified(obj);
        Ok(())
    }

    /// Set (insert or replace) an attribute; returns the previous value.
    pub fn set_attr(
        &mut self,
        id: ObjId,
        name: &str,
        attr: Attr,
    ) -> Result<Option<Attr>, WorldError> {
        self.ensure(id)?;
        let mut old = None;
        if let Some(o) = self.slot_mut(id) {
            old = o.attrs.set(name, attr);
        }
        self.touch_modified(id);
        Ok(old)
    }

    /// Remove an attribute; returns it if it was present.
    pub fn remove_attr(&mut self, id: ObjId, name: &str) -> Result<Option<Attr>, WorldError> {
        self.ensure(id)?;
        let mut old = None;
        if let Some(o) = self.slot_mut(id) {
            old = o.attrs.remove(name);
        }
        if old.is_some() {
            self.touch_modified(id);
        }
        Ok(old)
    }

    /// Rename an object. For actors this re-keys the player index and
    /// rejects names already claimed by a different actor.
    pub fn set_name(&mut self, id: ObjId, name: &str) -> Result<(), WorldError> {
        let is_actor = self
            .get(id)
            .ok_or(WorldError::NoSuchObj(id))?
            .kind
            .is_actor();
        if is_actor
            && let Some(&other) = self.player_index.get(&UncasedName::from(name))
            && other != id
        {
            return Err(WorldError::NameInUse(name.to_string()));
        }
        if let Some(o) = self.slot_mut(id) {
            o.name = name.to_string();
        }
        if is_actor {
            self.reindex_actor(id);
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Add an alias. For actors the alias joins the player index and must
    /// not collide with another actor's name or alias. Adding an alias the
    /// object already has (case-insensitively) is a no-op.
    pub fn add_alias(&mut self, id: ObjId, alias: &str) -> Result<(), WorldError> {
        let o = self.get(id).ok_or(WorldError::NoSuchObj(id))?;
        let is_actor = o.kind.is_actor();
        let key = UncasedName::from(alias);
        if o.aliases.iter().any(|a| UncasedName::from(a) == key) {
            return Ok(());
        }
        if is_actor
            && let Some(&other) = self.player_index.get(&key)
            && other != id
        {
            return Err(WorldError::NameInUse(alias.to_string()));
        }
        if let Some(o) = self.slot_mut(id) {
            o.aliases.push(smol_str::SmolStr::new(alias));
        }
        if is_actor {
            self.reindex_actor(id);
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Remove an alias (case-insensitively). Returns whether one was
    /// removed; the player index is re-keyed for actors.
    pub fn remove_alias(&mut self, id: ObjId, alias: &str) -> Result<bool, WorldError> {
        let is_actor = self
            .get(id)
            .ok_or(WorldError::NoSuchObj(id))?
            .kind
            .is_actor();
        let key = UncasedName::from(alias);
        let mut removed = false;
        if let Some(o) = self.slot_mut(id) {
            let before = o.aliases.len();
            o.aliases.retain(|a| UncasedName::from(a) != key);
            removed = o.aliases.len() != before;
        }
        if removed {
            if is_actor {
                self.reindex_actor(id);
            }
            self.touch_modified(id);
        }
        Ok(removed)
    }

    /// Assign (or clear) an object's zone, maintaining the membership
    /// index. The zone object must exist and may not be the object itself.
    /// (Whether it carries `ZONE_MASTER` is a permission question for the
    /// command layer, not an invariant here.)
    pub fn set_zone(&mut self, id: ObjId, zone: Option<ObjId>) -> Result<(), WorldError> {
        self.ensure(id)?;
        if let Some(z) = zone {
            self.ensure(z)?;
            if z == id {
                return Err(WorldError::Cycle { obj: id, dest: z });
            }
        }
        let old = self.get(id).and_then(|o| o.zone);
        if old == zone {
            return Ok(());
        }
        if let Some(oz) = old
            && let Some(set) = self.zone_members.get_mut(&oz)
        {
            set.remove(&id);
            if set.is_empty() {
                self.zone_members.remove(&oz);
            }
        }
        if let Some(z) = zone {
            self.zone_members.entry(z).or_default().insert(id);
        }
        if let Some(o) = self.slot_mut(id) {
            o.zone = zone;
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Set or clear one (or more OR-ed together) core flags.
    pub fn set_flag(&mut self, id: ObjId, flag: Flags, on: bool) -> Result<(), WorldError> {
        self.ensure(id)?;
        if let Some(o) = self.slot_mut(id) {
            o.flags.set(flag, on);
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Link (or unlink, with `None`) an exit's destination.
    pub fn link_exit(&mut self, exit: ObjId, dest: Option<ObjId>) -> Result<(), WorldError> {
        let kind = self.get(exit).ok_or(WorldError::NoSuchObj(exit))?.kind;
        if kind != ObjKind::Exit {
            return Err(WorldError::NotAnExit(exit));
        }
        if let Some(d) = dest {
            self.ensure(d)?;
        }
        if let Some(o) = self.slot_mut(exit) {
            o.destination = dest;
        }
        self.touch_modified(exit);
        Ok(())
    }

    /// Set or clear an object's home. The home must exist.
    pub fn set_home(&mut self, id: ObjId, home: Option<ObjId>) -> Result<(), WorldError> {
        self.ensure(id)?;
        if let Some(h) = home {
            self.ensure(h)?;
        }
        if let Some(o) = self.slot_mut(id) {
            o.home = home;
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Set or clear an object's `@parent`, rejecting parent loops.
    pub fn set_parent(&mut self, id: ObjId, parent: Option<ObjId>) -> Result<(), WorldError> {
        self.ensure(id)?;
        if let Some(p) = parent {
            self.ensure(p)?;
            let mut cur = Some(p);
            let mut hops = 0usize;
            while let Some(c) = cur {
                if c == id {
                    return Err(WorldError::ParentCycle { obj: id, parent: p });
                }
                cur = self.get(c).and_then(|o| o.parent);
                hops += 1;
                if hops > self.objs.len() {
                    break;
                }
            }
        }
        if let Some(o) = self.slot_mut(id) {
            o.parent = parent;
        }
        self.touch_modified(id);
        Ok(())
    }

    /// Transfer ownership. The new owner must exist.
    pub fn chown(&mut self, id: ObjId, new_owner: ObjId) -> Result<(), WorldError> {
        self.ensure(id)?;
        self.ensure(new_owner)?;
        if let Some(o) = self.slot_mut(id) {
            o.owner = new_owner;
        }
        self.touch_modified(id);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Marker flags (module-defined flags on objects)
    // ------------------------------------------------------------------

    /// Set or clear a marker flag (e.g. `"wod:kindred"`) on an object.
    /// Whether the marker is *registered* is the command layer's concern
    /// (`@set` consults the registry); the world just stores it.
    pub fn set_marker(&mut self, id: ObjId, name: &str, on: bool) -> Result<(), WorldError> {
        self.ensure(id)?;
        let mut changed = false;
        if let Some(o) = self.slot_mut(id) {
            changed = if on {
                o.markers.insert(SmolStr::new(name))
            } else {
                o.markers.remove(name)
            };
        }
        if changed {
            self.touch_modified(id);
        }
        Ok(())
    }

    /// Whether an object carries a marker flag (case-insensitive).
    #[must_use]
    pub fn has_marker(&self, id: ObjId, name: &str) -> bool {
        self.get(id)
            .is_some_and(|o| o.markers.iter().any(|m| m.eq_ignore_ascii_case(name)))
    }

    // ------------------------------------------------------------------
    // Scenes (transient; the tactics seam)
    // ------------------------------------------------------------------

    /// Create a scene (no hooks fire here — [`Ctx::start_scene`] is the
    /// module-facing path that does).
    ///
    /// [`Ctx::start_scene`]: crate::cmd::Ctx::start_scene
    pub fn scene_create(&mut self, kind: &str, ns: &str, members: BTreeSet<ObjId>) -> SceneId {
        let id = SceneId(self.next_scene_id);
        self.next_scene_id += 1;
        self.scenes.insert(
            id,
            Scene {
                id,
                kind: SmolStr::new(kind),
                ns: SmolStr::new(ns),
                members,
                ruleset_state_key: SmolStr::new(format!("scene:{}:state", id.0)),
            },
        );
        id
    }

    /// Look up an active scene.
    #[must_use]
    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.scenes.get(&id)
    }

    /// Iterate active scenes in id order.
    pub fn scenes(&self) -> impl Iterator<Item = &Scene> {
        self.scenes.values()
    }

    /// Remove a scene, dropping its ruleset state and map blob. Returns
    /// the removed scene ([`Ctx::end_scene`] is the module-facing path,
    /// which also fires hooks and queues persistence deletes).
    ///
    /// [`Ctx::end_scene`]: crate::cmd::Ctx::end_scene
    pub fn scene_remove(&mut self, id: SceneId) -> Option<Scene> {
        let scene = self.scenes.remove(&id)?;
        self.scene_state.retain(|(sid, _), _| *sid != id);
        self.scene_maps.remove(&id);
        Some(scene)
    }

    /// Add a member to a scene. Returns whether the scene exists and the
    /// member is newly added.
    pub fn scene_add_member(&mut self, id: SceneId, obj: ObjId) -> bool {
        self.scenes
            .get_mut(&id)
            .is_some_and(|s| s.members.insert(obj))
    }

    /// Remove a member from a scene. Returns whether it was a member.
    pub fn scene_remove_member(&mut self, id: SceneId, obj: ObjId) -> bool {
        self.scenes
            .get_mut(&id)
            .is_some_and(|s| s.members.remove(&obj))
    }

    /// One `(scene, obj)` ruleset-state value.
    #[must_use]
    pub fn scene_state(&self, scene: SceneId, obj: ObjId) -> Option<&serde_json::Value> {
        self.scene_state.get(&(scene, obj))
    }

    /// Set one `(scene, obj)` ruleset-state value. Returns `false` (and
    /// stores nothing) when the scene does not exist.
    pub fn scene_state_set(
        &mut self,
        scene: SceneId,
        obj: ObjId,
        value: serde_json::Value,
    ) -> bool {
        if !self.scenes.contains_key(&scene) {
            return false;
        }
        self.scene_state.insert((scene, obj), value);
        true
    }

    /// A serde snapshot of one scene's whole ruleset state: a JSON object
    /// keyed by dbref (`{"#5": {...}, ...}`) — what gets persisted per
    /// change.
    #[must_use]
    pub fn scene_state_snapshot(&self, scene: SceneId) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for ((sid, obj), v) in &self.scene_state {
            if *sid == scene {
                map.insert(obj.to_string(), v.clone());
            }
        }
        serde_json::Value::Object(map)
    }

    /// A scene's map blob.
    #[must_use]
    pub fn scene_map(&self, scene: SceneId) -> Option<&serde_json::Value> {
        self.scene_maps.get(&scene)
    }

    /// Set a scene's map blob (opaque to core). Returns `false` when the
    /// scene does not exist.
    pub fn scene_map_set(&mut self, scene: SceneId, value: serde_json::Value) -> bool {
        if !self.scenes.contains_key(&scene) {
            return false;
        }
        self.scene_maps.insert(scene, value);
        true
    }

    // ------------------------------------------------------------------
    // Typed module state slots
    // ------------------------------------------------------------------

    /// Read the module state slot for `T`.
    #[must_use]
    pub fn module_state<T: Any + Send>(&self) -> Option<&T> {
        self.module_state.get::<T>()
    }

    /// Mutably access the module state slot for `T`.
    pub fn module_state_mut<T: Any + Send>(&mut self) -> Option<&mut T> {
        self.module_state.get_mut::<T>()
    }

    /// Install (or replace) the state slot for `T` directly (modules may
    /// do this from `on_world_loaded` to seed loaded state).
    pub fn module_state_insert<T: Any + Send + Clone>(&mut self, value: T) {
        self.module_state.insert(value);
    }

    /// Install a collected [`StateInit`] (engine construction).
    pub fn module_state_apply(&mut self, init: StateInit) {
        self.module_state.apply(init);
    }

    // ------------------------------------------------------------------
    // Input-context stacks (transient parser state)
    // ------------------------------------------------------------------

    /// Input-context stack depth for an actor.
    #[must_use]
    pub fn input_depth(&self, actor: ObjId) -> usize {
        self.input_stacks.depth(actor)
    }

    /// Push an input context (tagged with the pushing module's identity) on
    /// top of an actor's stack (prompt emission is the command layer's job
    /// — [`Ctx::push_input_context`]).
    ///
    /// [`Ctx::push_input_context`]: crate::cmd::Ctx::push_input_context
    pub fn push_input_context(
        &mut self,
        actor: ObjId,
        module: Option<SmolStr>,
        cx: Box<dyn InputContext>,
    ) {
        self.input_stacks.push(actor, module, cx);
    }

    /// Pop an actor's top input context, if any.
    pub fn pop_input_context(&mut self, actor: ObjId) -> Option<crate::module::StackedContext> {
        self.input_stacks.pop(actor)
    }

    /// Re-insert a context at `index` in an actor's stack (dispatch
    /// internals: put a fed context back beneath anything it pushed).
    pub fn restore_input_context_at(
        &mut self,
        actor: ObjId,
        index: usize,
        cx: crate::module::StackedContext,
    ) {
        self.input_stacks.insert_at(actor, index, cx);
    }

    /// The prompt of an actor's top input context, if any.
    #[must_use]
    pub fn top_input_prompt(&self, actor: ObjId) -> Option<PromptSpec> {
        self.input_stacks.top(actor).map(|c| c.prompt(self))
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    fn ensure(&self, id: ObjId) -> Result<(), WorldError> {
        if self.get(id).is_some() {
            Ok(())
        } else {
            Err(WorldError::NoSuchObj(id))
        }
    }

    /// Mutable slot access WITHOUT dirty-marking (internals mark dirty via
    /// [`World::touch_modified`] so timestamps stay consistent).
    fn slot_mut(&mut self, id: ObjId) -> Option<&mut Obj> {
        self.objs.get_mut(id.0 as usize).and_then(Option::as_mut)
    }

    /// Stamp `modified` and mark dirty.
    fn touch_modified(&mut self, id: ObjId) {
        let now = self.now;
        if let Some(o) = self.slot_mut(id) {
            o.modified = now;
        }
        self.dirty.insert(id);
    }

    /// Rebuild the player-index entries for one actor from its current
    /// name + aliases. Existing claims by OTHER actors are never
    /// overwritten (collisions are rejected before mutation).
    fn reindex_actor(&mut self, id: ObjId) {
        self.player_index.retain(|_, v| *v != id);
        let keys = self
            .get(id)
            .filter(|o| o.kind.is_actor() && !o.flags.contains(Flags::GOING))
            .map(|o| {
                let mut keys = vec![UncasedName::from(o.name.as_str())];
                keys.extend(o.aliases.iter().map(UncasedName::from));
                keys
            });
        if let Some(keys) = keys {
            for k in keys {
                self.player_index.entry(k).or_insert(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::Attr;
    use crate::flags::Role;

    #[test]
    fn new_minimal_invariants() {
        let w = World::new_minimal();
        let limbo = w.get(LIMBO).unwrap();
        let god = w.get(GOD).unwrap();
        assert_eq!(limbo.kind, ObjKind::Room);
        assert_eq!(god.kind, ObjKind::Player);
        assert_eq!(god.location, Some(LIMBO));
        assert_eq!(god.home, Some(LIMBO));
        assert_eq!(limbo.contents, vec![GOD]);
        assert_eq!(w.resolve_player("god"), Some(GOD));
        assert_eq!(w.resolve_player("GOD"), Some(GOD));
        assert_eq!(god.role(), Role::God);
        assert_eq!(god.owner, GOD);
    }

    #[test]
    fn game_clock_reads_ic_time_from_now() {
        let mut w = World::new_minimal();
        // Default clock is 1:1 with OOC time and phase 0.
        assert_eq!(w.game_clock(), GameClock::default());
        // Install a 4x clock anchored so OOC time 0 reads a fixed IC date.
        let epoch = crate::clock::CivilDate::parse("2100-03-01 00:00").unwrap();
        w.set_game_clock(GameClock {
            ic_ratio: 4,
            epoch_offset: epoch.to_ic_seconds(),
        });
        // At OOC now 0 the IC clock reads exactly the epoch.
        w.set_now(0);
        assert_eq!(w.ic_datetime(), "March 1, 2100, 00:00");
        assert_eq!((w.ic_month(), w.ic_day(), w.ic_year()), (3, 1, 2100));
        assert_eq!(w.season(), Season::Spring);
        assert_eq!(w.time_of_day(), TimeOfDay::Night);
        // Six OOC hours -> 24 IC hours -> next IC day; season/tod track it.
        w.set_now(6 * 3_600);
        assert_eq!(w.ic_day(), 2);
        assert_eq!(w.time_of_day(), TimeOfDay::Night);
        assert_eq!(w.ic_hour(), 0);
    }

    #[test]
    fn create_wires_contents_and_exits() {
        let mut w = World::new_minimal();
        w.set_now(1000);
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let chest = w.create(ObjKind::Thing, "chest", GOD, Some(hall)).unwrap();
        let north = w.create(ObjKind::Exit, "north;n", GOD, Some(hall)).unwrap();
        assert_eq!(w.contents_of(hall), &[chest]);
        assert_eq!(w.exits_of(hall), &[north]);
        assert_eq!(w.get(chest).unwrap().location, Some(hall));
        assert_eq!(w.get(north).unwrap().location, Some(hall));
        assert_eq!(w.get(chest).unwrap().created, 1000);
        // Exits are not contents; things are not exits.
        assert!(w.contents_of(hall).iter().all(|&c| c != north));
        assert!(w.exits_of(hall).iter().all(|&e| e != chest));
    }

    #[test]
    fn create_player_self_owns_and_rejects_duplicates() {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        assert_eq!(w.get(nick).unwrap().owner, nick);
        assert_eq!(w.get(nick).unwrap().home, Some(LIMBO));
        assert_eq!(
            w.create(ObjKind::Player, "nick", GOD, Some(LIMBO)),
            Err(WorldError::NameInUse("nick".into()))
        );
        // NPCs collide with players too (shared index), and keep their owner.
        assert_eq!(
            w.create(ObjKind::Npc, "NICK", GOD, Some(LIMBO)),
            Err(WorldError::NameInUse("NICK".into()))
        );
        let barkeep = w.create(ObjKind::Npc, "Barkeep", GOD, Some(LIMBO)).unwrap();
        assert_eq!(w.get(barkeep).unwrap().owner, GOD);
        assert_eq!(w.resolve_player("barkeep"), Some(barkeep));
    }

    #[test]
    fn create_validates_owner_and_location() {
        let mut w = World::new_minimal();
        assert_eq!(
            w.create(ObjKind::Thing, "x", ObjId(99), Some(LIMBO)),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );
        assert_eq!(
            w.create(ObjKind::Thing, "x", GOD, Some(ObjId(99))),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );
    }

    #[test]
    fn move_updates_both_sides() {
        let mut w = World::new_minimal();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO)).unwrap();
        assert!(w.contents_of(LIMBO).contains(&ball));
        w.move_to(ball, hall).unwrap();
        assert!(!w.contents_of(LIMBO).contains(&ball));
        assert_eq!(w.contents_of(hall), &[ball]);
        assert_eq!(w.get(ball).unwrap().location, Some(hall));
        // Exit moves land in the exits list of the new source room.
        let door = w.create(ObjKind::Exit, "door", GOD, Some(LIMBO)).unwrap();
        w.move_to(door, hall).unwrap();
        assert!(w.exits_of(LIMBO).is_empty());
        assert_eq!(w.exits_of(hall), &[door]);
    }

    #[test]
    fn move_rejects_cycles() {
        let mut w = World::new_minimal();
        let bag = w.create(ObjKind::Thing, "bag", GOD, Some(LIMBO)).unwrap();
        let box_ = w.create(ObjKind::Thing, "box", GOD, Some(bag)).unwrap();
        let pouch = w.create(ObjKind::Thing, "pouch", GOD, Some(box_)).unwrap();
        // Into itself.
        assert!(matches!(w.move_to(bag, bag), Err(WorldError::Cycle { .. })));
        // Direct cycle.
        assert!(matches!(
            w.move_to(bag, box_),
            Err(WorldError::Cycle { .. })
        ));
        // Transitive cycle: bag -> pouch, but pouch is inside box inside bag.
        assert!(matches!(
            w.move_to(bag, pouch),
            Err(WorldError::Cycle { .. })
        ));
        // The failed moves changed nothing.
        assert_eq!(w.get(bag).unwrap().location, Some(LIMBO));
        assert_eq!(w.contents_of(bag), &[box_]);
    }

    #[test]
    fn destroy_detaches_and_sweep_tombstones() {
        let mut w = World::new_minimal();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let north = w.create(ObjKind::Exit, "north", GOD, Some(hall)).unwrap();
        let chest = w.create(ObjKind::Thing, "chest", GOD, Some(hall)).unwrap();
        let coin = w.create(ObjKind::Thing, "coin", GOD, Some(hall)).unwrap();
        w.set_home(coin, Some(LIMBO)).unwrap();
        let den = w.create(ObjKind::Room, "Den", GOD, None).unwrap();
        w.set_home(chest, Some(den)).unwrap();

        w.destroy(hall).unwrap();
        // Contents went home; homeless would go to Limbo.
        assert_eq!(w.get(chest).unwrap().location, Some(den));
        assert_eq!(w.get(coin).unwrap().location, Some(LIMBO));
        assert!(w.contents_of(den).contains(&chest));
        assert!(w.contents_of(LIMBO).contains(&coin));
        // The room's exits die with it.
        assert!(w.get(north).unwrap().flags.contains(Flags::GOING));
        assert!(w.get(hall).unwrap().flags.contains(Flags::GOING));

        let next_before = w.create(ObjKind::Thing, "probe", GOD, None).unwrap();
        let swept = w.sweep();
        assert!(swept.contains(&hall) && swept.contains(&north));
        assert!(w.get(hall).is_none());
        assert!(w.get(north).is_none());
        // Ids are never recycled: new ids keep counting upward.
        let after = w.create(ObjKind::Thing, "probe2", GOD, None).unwrap();
        assert!(after.0 > next_before.0);
    }

    #[test]
    fn destroy_thing_without_home_sends_contents_to_limbo() {
        let mut w = World::new_minimal();
        let bag = w.create(ObjKind::Thing, "bag", GOD, Some(LIMBO)).unwrap();
        let gem = w.create(ObjKind::Thing, "gem", GOD, Some(bag)).unwrap();
        w.destroy(bag).unwrap();
        assert_eq!(w.get(gem).unwrap().location, Some(LIMBO));
        assert!(w.contents_of(LIMBO).contains(&gem));
        // Holder no longer lists the destroyed bag.
        assert!(!w.contents_of(LIMBO).contains(&bag));
    }

    #[test]
    fn destroy_protects_god_and_limbo_and_clears_player_index() {
        let mut w = World::new_minimal();
        assert_eq!(w.destroy(GOD), Err(WorldError::Protected(GOD)));
        assert_eq!(w.destroy(LIMBO), Err(WorldError::Protected(LIMBO)));
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
        w.add_alias(nick, "N").unwrap();
        w.destroy(nick).unwrap();
        assert_eq!(w.resolve_player("Nick"), None);
        assert_eq!(w.resolve_player("n"), None);
        // Name becomes available again once its holder is gone.
        w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();
    }

    #[test]
    fn zone_index_maintained() {
        let mut w = World::new_minimal();
        let zmo = w
            .create(ObjKind::Thing, "Downtown ZMO", GOD, Some(LIMBO))
            .unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        let r1 = w.create(ObjKind::Room, "Bar", GOD, None).unwrap();
        let r2 = w.create(ObjKind::Room, "Alley", GOD, None).unwrap();
        w.set_zone(r1, Some(zmo)).unwrap();
        w.set_zone(r2, Some(zmo)).unwrap();
        let members: Vec<_> = w.zone_members_of(zmo).collect();
        assert_eq!(members, vec![r1, r2]);

        // Re-zoning moves membership.
        let zmo2 = w
            .create(ObjKind::Thing, "Uptown ZMO", GOD, Some(LIMBO))
            .unwrap();
        w.set_zone(r1, Some(zmo2)).unwrap();
        assert_eq!(w.zone_members_of(zmo).collect::<Vec<_>>(), vec![r2]);
        assert_eq!(w.zone_members_of(zmo2).collect::<Vec<_>>(), vec![r1]);

        // Clearing removes membership.
        w.set_zone(r2, None).unwrap();
        assert_eq!(w.zone_members_of(zmo).count(), 0);

        // An object cannot be its own zone.
        assert!(matches!(
            w.set_zone(zmo, Some(zmo)),
            Err(WorldError::Cycle { .. })
        ));

        // Destroying a ZMO releases its members.
        w.destroy(zmo2).unwrap();
        assert_eq!(w.get(r1).unwrap().zone, None);
        assert_eq!(w.zone_members_of(zmo2).count(), 0);
    }

    #[test]
    fn rename_and_alias_update_player_index() {
        let mut w = World::new_minimal();
        let nick = w
            .create(ObjKind::Player, "Nicholas", GOD, Some(LIMBO))
            .unwrap();
        w.add_alias(nick, "Nick").unwrap();
        assert_eq!(w.resolve_player("nick"), Some(nick));
        assert_eq!(w.resolve_player("NICHOLAS"), Some(nick));

        w.set_name(nick, "York").unwrap();
        assert_eq!(w.resolve_player("Nicholas"), None);
        assert_eq!(w.resolve_player("york"), Some(nick));
        assert_eq!(w.resolve_player("nick"), Some(nick)); // alias survives rename

        assert!(w.remove_alias(nick, "NICK").unwrap());
        assert_eq!(w.resolve_player("nick"), None);
        assert!(!w.remove_alias(nick, "nick").unwrap());

        // Collisions rejected for both name and alias.
        let bob = w.create(ObjKind::Player, "Bob", GOD, Some(LIMBO)).unwrap();
        assert_eq!(
            w.set_name(bob, "YORK"),
            Err(WorldError::NameInUse("YORK".into()))
        );
        assert_eq!(
            w.add_alias(bob, "york"),
            Err(WorldError::NameInUse("york".into()))
        );
        // Renaming to your own name (case change) is fine.
        w.set_name(bob, "BOB").unwrap();
        assert_eq!(w.resolve_player("bob"), Some(bob));
    }

    #[test]
    fn non_actor_names_are_not_indexed() {
        let mut w = World::new_minimal();
        let chest = w.create(ObjKind::Thing, "chest", GOD, Some(LIMBO)).unwrap();
        assert_eq!(w.resolve_player("chest"), None);
        // Things may share names with players.
        let p = w
            .create(ObjKind::Player, "Chest", GOD, Some(LIMBO))
            .unwrap();
        assert_eq!(w.resolve_player("chest"), Some(p));
        let _ = chest;
    }

    #[test]
    fn attr_inheritance_walks_parent_chain() {
        let mut w = World::new_minimal();
        let grandpa = w.create(ObjKind::Thing, "grandpa", GOD, None).unwrap();
        let parent = w.create(ObjKind::Thing, "parent", GOD, None).unwrap();
        let child = w.create(ObjKind::Thing, "child", GOD, None).unwrap();
        w.set_parent(parent, Some(grandpa)).unwrap();
        w.set_parent(child, Some(parent)).unwrap();

        w.set_attr(grandpa, "desc", Attr::text("from grandpa", GOD))
            .unwrap();
        assert_eq!(
            w.attr(child, "DESC").unwrap().as_text(),
            Some("from grandpa")
        );

        // Nearer definitions win.
        w.set_attr(parent, "desc", Attr::text("from parent", GOD))
            .unwrap();
        assert_eq!(
            w.attr(child, "desc").unwrap().as_text(),
            Some("from parent")
        );
        w.set_attr(child, "desc", Attr::text("own", GOD)).unwrap();
        assert_eq!(w.attr(child, "desc").unwrap().as_text(), Some("own"));
    }

    #[test]
    fn attr_no_inherit_is_skipped_but_walk_continues() {
        let mut w = World::new_minimal();
        let grandpa = w.create(ObjKind::Thing, "grandpa", GOD, None).unwrap();
        let parent = w.create(ObjKind::Thing, "parent", GOD, None).unwrap();
        let child = w.create(ObjKind::Thing, "child", GOD, None).unwrap();
        w.set_parent(parent, Some(grandpa)).unwrap();
        w.set_parent(child, Some(parent)).unwrap();

        w.set_attr(grandpa, "secret", Attr::text("inheritable", GOD))
            .unwrap();
        let mut private = Attr::text("private to parent", GOD);
        private.flags = AttrFlags::NO_INHERIT;
        w.set_attr(parent, "secret", private).unwrap();

        // Child skips the parent's NO_INHERIT copy but still sees grandpa's.
        assert_eq!(
            w.attr(child, "secret").unwrap().as_text(),
            Some("inheritable")
        );
        // The parent itself sees its own attribute (NO_INHERIT only blocks
        // inheritance, not self-access).
        assert_eq!(
            w.attr(parent, "secret").unwrap().as_text(),
            Some("private to parent")
        );
    }

    #[test]
    fn attr_walk_depth_bounded() {
        let mut w = World::new_minimal();
        // chain[0] <- chain[1] <- ... <- chain[12]; attr lives on the top.
        let mut chain = Vec::new();
        for i in 0..13 {
            chain.push(
                w.create(ObjKind::Thing, &format!("link{i}"), GOD, None)
                    .unwrap(),
            );
        }
        for pair in chain.windows(2) {
            w.set_parent(pair[0], Some(pair[1])).unwrap();
        }
        w.set_attr(chain[12], "deep", Attr::text("too far", GOD))
            .unwrap();
        // 12 hops from chain[0]: beyond MAX_PARENT_DEPTH (10) -> invisible.
        assert!(w.attr(chain[0], "deep").is_none());
        // 10 hops from chain[2]: exactly at the bound -> visible.
        assert_eq!(w.attr(chain[2], "deep").unwrap().as_text(), Some("too far"));
        // 11 hops from chain[1]: one past -> invisible.
        assert!(w.attr(chain[1], "deep").is_none());
    }

    #[test]
    fn attr_walk_survives_parent_cycle() {
        let mut w = World::new_minimal();
        let a = w.create(ObjKind::Thing, "a", GOD, None).unwrap();
        let b = w.create(ObjKind::Thing, "b", GOD, None).unwrap();
        w.set_parent(a, Some(b)).unwrap();
        // set_parent rejects closing the loop...
        assert_eq!(
            w.set_parent(b, Some(a)),
            Err(WorldError::ParentCycle { obj: b, parent: a })
        );
        // ...but even a forced cycle terminates thanks to the depth bound.
        w.get_mut(b).unwrap().parent = Some(a);
        assert!(w.attr(a, "nothing").is_none());
    }

    #[test]
    fn dirty_tracking_and_take_dirty() {
        let mut w = World::new_minimal();
        let _ = w.take_dirty(); // clear bootstrap dirt
        assert!(w.take_dirty().is_empty());

        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO)).unwrap();
        let d = w.take_dirty();
        assert!(
            d.contains(&ball) && d.contains(&LIMBO),
            "creation dirties holder too: {d:?}"
        );

        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let _ = w.take_dirty();
        w.move_to(ball, hall).unwrap();
        let d = w.take_dirty();
        assert!(d.contains(&ball) && d.contains(&hall) && d.contains(&LIMBO));

        // get_mut marks dirty; accessors don't.
        let _ = w.get(ball);
        assert!(w.take_dirty().is_empty());
        let _ = w.get_mut(ball);
        assert_eq!(w.take_dirty().into_iter().collect::<Vec<_>>(), vec![ball]);
    }

    #[test]
    fn modified_timestamp_follows_now() {
        let mut w = World::new_minimal();
        let ball = w.create(ObjKind::Thing, "ball", GOD, Some(LIMBO)).unwrap();
        w.set_now(5_000);
        w.set_attr(ball, "desc", Attr::text("round", GOD)).unwrap();
        let o = w.get(ball).unwrap();
        assert_eq!(o.created, 0);
        assert_eq!(o.modified, 5_000);
    }

    #[test]
    fn link_exit_and_misc_mutators_validate() {
        let mut w = World::new_minimal();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let north = w.create(ObjKind::Exit, "north", GOD, Some(LIMBO)).unwrap();
        let rock = w.create(ObjKind::Thing, "rock", GOD, Some(LIMBO)).unwrap();

        w.link_exit(north, Some(hall)).unwrap();
        assert_eq!(w.get(north).unwrap().destination, Some(hall));
        w.link_exit(north, None).unwrap();
        assert_eq!(w.get(north).unwrap().destination, None);
        assert_eq!(
            w.link_exit(rock, Some(hall)),
            Err(WorldError::NotAnExit(rock))
        );
        assert_eq!(
            w.link_exit(north, Some(ObjId(99))),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );

        assert_eq!(
            w.set_home(rock, Some(ObjId(99))),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );
        assert_eq!(
            w.chown(rock, ObjId(99)),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );
        w.chown(rock, GOD).unwrap();
        assert_eq!(w.get(rock).unwrap().owner, GOD);

        w.set_flag(rock, Flags::STICKY, true).unwrap();
        assert!(w.get(rock).unwrap().flags.contains(Flags::STICKY));
        w.set_flag(rock, Flags::STICKY, false).unwrap();
        assert!(!w.get(rock).unwrap().flags.contains(Flags::STICKY));
    }

    #[test]
    fn from_objects_round_trips_a_built_world() {
        // Build a world with structure: rooms, an exit, an actor with an
        // alias, a zone, a destroyed-and-swept object (tombstone gap).
        let mut w = World::new_minimal();
        let hall = w.create(ObjKind::Room, "Hall", GOD, None).unwrap();
        let north = w.create(ObjKind::Exit, "north;n", GOD, Some(hall)).unwrap();
        w.link_exit(north, Some(LIMBO)).unwrap();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(hall)).unwrap();
        w.add_alias(nick, "Nicky").unwrap();
        let sword = w.create(ObjKind::Thing, "sword", nick, Some(nick)).unwrap();
        let zmo = w.create(ObjKind::Thing, "ZMO", GOD, Some(LIMBO)).unwrap();
        w.set_flag(zmo, Flags::ZONE_MASTER, true).unwrap();
        w.set_zone(hall, Some(zmo)).unwrap();
        let doomed = w.create(ObjKind::Thing, "doomed", GOD, Some(hall)).unwrap();
        w.destroy(doomed).unwrap();
        w.sweep();

        let objects: Vec<Obj> = w.iter().cloned().collect();
        let loaded = World::from_objects(objects, w.next_id()).unwrap();

        // Same ids resolve; indexes rebuilt; tombstone preserved.
        assert_eq!(loaded.resolve_player("nick"), Some(nick));
        assert_eq!(loaded.resolve_player("NICKY"), Some(nick));
        assert_eq!(loaded.contents_of(nick), &[sword]);
        assert!(loaded.contents_of(hall).contains(&nick));
        assert_eq!(loaded.exits_of(hall), &[north]);
        assert_eq!(loaded.get(north).unwrap().destination, Some(LIMBO));
        assert_eq!(loaded.zone_members_of(zmo).collect::<Vec<_>>(), vec![hall]);
        assert!(loaded.get(doomed).is_none(), "tombstone survives the load");
        // Loading is not a mutation.
        let mut loaded = loaded;
        assert!(loaded.take_dirty().is_empty());
        // Allocation resumes exactly where it stopped.
        let fresh = loaded.create(ObjKind::Thing, "fresh", GOD, None).unwrap();
        assert_eq!(fresh.0, w.next_id());
    }

    #[test]
    fn from_objects_validates() {
        let base = || {
            let w = World::new_minimal();
            w.iter().cloned().collect::<Vec<Obj>>()
        };
        // Duplicate ids.
        let mut objs = base();
        objs.push(objs[0].clone());
        assert_eq!(
            World::from_objects(objs, 2).unwrap_err(),
            WorldError::DuplicateId(LIMBO)
        );
        // next_id too small.
        assert_eq!(
            World::from_objects(base(), 1).unwrap_err(),
            WorldError::BadNextId {
                next_id: 1,
                id: GOD
            }
        );
        // Dangling location.
        let mut objs = base();
        objs[1].location = Some(ObjId(9));
        assert_eq!(
            World::from_objects(objs, 2).unwrap_err(),
            WorldError::NoSuchObj(ObjId(9))
        );
        // Actor name collision.
        let mut objs = base();
        let mut clone_god = Obj::new(ObjId(2), ObjKind::Player, "GOD", ObjId(2), 0);
        clone_god.location = Some(LIMBO);
        objs.push(clone_god);
        assert_eq!(
            World::from_objects(objs, 3).unwrap_err(),
            WorldError::NameInUse("GOD".into())
        );
    }

    #[test]
    fn session_stats_track_connect_and_input_times() {
        let mut w = World::new_minimal();
        let nick = w.create(ObjKind::Player, "Nick", GOD, Some(LIMBO)).unwrap();

        w.set_now(1_000);
        w.session_start(nick);
        assert_eq!(
            w.session_stat(nick),
            Some(SessionStat {
                connect_time: 1_000,
                last_input: 1_000
            })
        );

        // Input stamps last_input; connect_time is untouched.
        w.set_now(1_040);
        w.session_input(nick);
        let s = w.session_stat(nick).unwrap();
        assert_eq!((s.connect_time, s.last_input), (1_000, 1_040));

        // A second session_start (multi-connect) never resets the times.
        w.set_now(1_100);
        w.session_start(nick);
        assert_eq!(w.session_stat(nick).unwrap().connect_time, 1_000);

        // Ending forgets; input without a session is a no-op.
        w.session_end(nick);
        assert_eq!(w.session_stat(nick), None);
        w.session_input(nick);
        assert_eq!(w.session_stat(nick), None);

        // Missing objects never get stats; destroy clears them.
        w.session_start(ObjId(99));
        assert_eq!(w.session_stat(ObjId(99)), None);
        w.session_start(nick);
        w.destroy(nick).unwrap();
        assert_eq!(w.session_stat(nick), None);
    }

    #[test]
    fn correlation_ids_are_monotonic() {
        let mut w = World::new_minimal();
        assert_eq!(w.next_correlation_id(), 0);
        assert_eq!(w.next_correlation_id(), 1);
        assert_eq!(w.next_correlation_id(), 2);
    }

    #[test]
    fn set_and_remove_attr_round_trip() {
        let mut w = World::new_minimal();
        let rock = w.create(ObjKind::Thing, "rock", GOD, Some(LIMBO)).unwrap();
        assert!(
            w.set_attr(rock, "desc", Attr::text("grey", GOD))
                .unwrap()
                .is_none()
        );
        let old = w.set_attr(rock, "DESC", Attr::text("mossy", GOD)).unwrap();
        assert_eq!(old.unwrap().as_text(), Some("grey"));
        let gone = w.remove_attr(rock, "Desc").unwrap();
        assert_eq!(gone.unwrap().as_text(), Some("mossy"));
        assert!(w.remove_attr(rock, "desc").unwrap().is_none());
        assert_eq!(
            w.set_attr(ObjId(99), "x", Attr::text("y", GOD)),
            Err(WorldError::NoSuchObj(ObjId(99)))
        );
    }
}
