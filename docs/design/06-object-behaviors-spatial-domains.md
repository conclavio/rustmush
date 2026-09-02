# Object behaviors, spatial relationships, and simulation domains

**Status:** Draft 0.1 for design review  
**Scope:** Object-attached game behavior, controllable units, maps/domains, interiors,
transport, attachments, deterministic ticking, and the TacSim first vertical slice  
**Applies to:** `mush-core`, a proposed `mush-sim` foundation crate, `mush-scenes`,
`mush-proto`, and `mush-tactics`

This document starts the dedicated TacSim design conversation deferred by
`docs/PLAN.md`. It defines a generic model first and uses TacSim as its initial proving
system. The model must also support Shadowrun vehicles, drones, ships, mounts, static weapon
platforms, and other rulesets without putting any of those rules into `mush-core`.

**This is the spatial-foundation half of a two-document design.** It covers behaviors, the
placement graph, domains and their clocks, atomic transitions, persistence, presentation, and scene
journaling — everything the world needs to hold units in space. The question of *who or what
controls a unit when a human is not present, and how that controller reasons* — link-loss
automation, allegiance, typed orders, hybrid tactical AI, and NPC-driven traffic — was originally
appended to this draft and now lives in its companion,
`docs/design/07-npc-control-and-tactical-ai.md`. The foundation here does not depend on that layer
and should ship first.

The design is inspired by player-visible MU* conventions: ordinary MUSH objects acquire typed
behavior, commands are supplied by the object being operated rather than by a global game command
set, and units interact inside a shared map-like simulation boundary. This is an independent
RustMUSH design. No third-party implementation, persistence format, source structure, algorithms,
tables, or code are part of this specification.

---

## 1. Goals

1. Attach namespaced, typed behavior to ordinary RustMUSH objects.
2. Let attached behavior contribute contextual commands such as `STARTUP`, `SPEED`, `HEADING`,
   `TACTICAL`, and `VIEW` without registering those verbs globally.
3. Represent direct placement, interiors, cargo, crew stations, and external attachments without
   treating all of them as ordinary MUSH containment.
4. Make a simulation domain the deterministic clock and interaction boundary for its projected
   participants.
5. Allow a moving carrier to carry rooms, units, cargo, crew, and attached units across a map or
   between maps without rewriting every descendant's position.
6. Keep durable narrative scenes distinct from persistent spatial domains while allowing a scene
   to bind to a domain and journal its mechanical events.
7. Keep generic infrastructure independent of TacSim rules and data.
8. Preserve the single-writer `World` architecture, write-through command atomicity, bounded work,
   transport-neutral output, and module isolation already implemented by RustMUSH.

## 2. Non-goals for the first vertical slice

- A complete TacSim rules implementation.
- Reproducing another server's binary formats, commands byte-for-byte, formulas, or internal data
  structures.
- Arbitrary user-loaded native plugins. Modules remain statically linked Rust crates.
- An actor or Tokio task per unit. The world task remains the sole state writer.
- A universal physics engine. Coordinate systems, movement rules, damage, sensors, and weapons stay
  ruleset-owned.
- Making all ordinary rooms simulation domains.
- Interior deck-plan combat in the first slice. The model must permit it later.

---

## 3. Architectural boundaries

The system has four related but distinct layers.

```text
MUSH world object
    identity, owner, location/contents, attributes, locks
             |
             +-- attached behavior bindings
             |       typed commands and ruleset-owned durable state
             |
             +-- spatial parent
                     direct domain placement, interior, or attachment
                              |
                              +-- effective simulation domain
                                      clock, membership projection,
                                      intents, LOS/sensors, tick commits

Narrative scene ----------------------------------------------------+
    participants, facilitator, transcript, fictional time           |
    may bind to a simulation domain and record domain events <-------+
```

### 3.1 `mush-core`

`mush-core` owns only facilities that must participate directly in object persistence or command
dispatch:

- durable behavior bindings on `Obj`;
- behavior type registration and lifecycle validation;
- contextual behavior-command resolution;
- generic command-subject links published by infrastructure modules;
- the hooks needed to keep behavior and spatial state consistent across move and destroy attempts.

It does not know what a mech, vehicle, hex, weapon, or line of sight is.

### 3.2 Proposed `mush-sim`

`mush-sim` is a foundational crate comparable to `mush-ruleset`, not a game system. It owns:

- durable simulation-domain identities and records;
- the authoritative spatial-parent graph;
- interiors, compartments, mount points, control stations, and assignments;
- derived locality and tactical-presence indexes;
- atomic placement transitions;
- domain clocks, intent buffers, tick execution, checkpoints, and replay;
- generic domain inspection and recovery tools.

Other ruleset crates may depend on `mush-sim`. `mush-sim` depends on `mush-core` and
`mush-proto`, never on a ruleset.

### 3.3 `mush-tactics` (generic tactical layer) and `ruleset-tacsim` (setting rules)

`mush-tactics` is the reusable, ruleset-independent tactical engine on top of `mush-sim`. It owns
the generic machinery, never any setting's rules:

- the `TacticalUnit` composition — body model, mobility, footprint, stations, equipment, and
  tactical role — a unit is composed capabilities, never a subclass (a 'Mech, tank, aerospace
  fighter, and taxi are all `TacticalUnit`s);
- `MobilityProfile` with registered mode/medium/locomotion ids for multimodal units, leaving
  terrain-cost and kinematics maths to the ruleset;
- contacts, formations, missions, transport, and the NPC-controller / legal-action-projection seam;
- the domain-driver interface and the rendering projection tactical views consume.

`ruleset-tacsim` is the game ruleset adapter — deliberately generic (our own rules, trademark-free;
not any published system). It registers the concrete setting data and formulas through
`mush-tactics`:

- unit classes, templates, and construction;
- hex-map terrain and coordinate data;
- armor, internals, heat, sensors, weapons, and damage formulas;
- initiative / turn structure and to-hit / gunnery rolls;
- bay, mount, and deployment validation rules.

`warfare` (the future campaign tier — supply, patrols, territory) sits above both.

### 3.4 `mush-scenes`

`mush-scenes` remains the owner of durable narrative scenes and Timestop. A domain is where and how
simulation advances; a scene is the bounded piece of play being recorded. Either may exist without
the other.

Examples:

- A persistent world map may exist for months and host many engagement scenes over time.
- A freeform bar scene needs no simulation domain.
- A WoD Timestop is a turn-based scene in an ordinary room.
- A TacSim engagement is a real-time scene bound to a fixed-step domain.

The boot-scoped `mush_core::SceneId`, per-scene state store, and map blob currently exercised by the
TacSim stub are prototype seams, not the final durable spatial model. The stub stays green while
the new APIs are built, then migrates to them. The durable `mush-scenes::SceneUid` remains the
narrative identity.

### 3.5 Rejected shortcuts

- **A writable `&xtype` attribute as authority:** it bypasses lifecycle, compatibility, persistence,
  and cleanup validation. A compatibility command may wrap `@behavior/add`, but the typed binding is
  authoritative.
- **Register every vehicle verb globally:** it exposes commands where no applicable object exists
  and makes collisions between rulesets inevitable.
- **Use `Obj.location` as tactical placement:** it cannot distinguish cargo, exposed attachments,
  crew, moving interiors, or ruleset coordinates.
- **Persist both `unit.map_id` and `map.members` as peers:** partial writes and recovery can make the
  two disagree. One placement graph is authoritative.
- **Make every unit or map an async actor:** it breaks the world's existing transaction and ordering
  guarantees without useful MU*-scale throughput.
- **Treat a runtime scene as a persistent map:** engagements have bounded narrative lifetimes;
  terrain and spatial placement often outlive many engagements.

---

## 4. First-class object behaviors

### 4.1 Durable binding

Add a behavior map to `mush_core::Obj` with `#[serde(default)]` for backward-compatible loading:

```rust
pub struct BehaviorBinding {
    pub schema_version: u32,
    pub attached_by: ObjId,
    pub attached_at: i64,
}

pub struct Obj {
    // existing fields...
    #[serde(default)]
    pub behaviors: BTreeMap<BehaviorId, BehaviorBinding>,
}
```

`BehaviorId` is a validated namespaced identifier such as:

- `sim.domain`
- `sim.participant`
- `sim.interior`
- `sim.control_station`
- `tactics.map`
- `tactics.unit`
- `tactics.unit`
- `shadowrun.vehicle`

The binding declares that behavior exists on the object. Large or frequently changing behavior
state remains in the owning module's persisted namespace, for example `mod:tactics:unit:123`.
Markers remain lightweight flags and are not used as a substitute for behavior lifecycle.

**Grounding:** `Obj` is persisted as one whole-object JSON doc (`obj:{id}`) and deep-cloned each
persist cycle, but an empty `BTreeMap` neither heap-allocates nor costs anything to clone, so an
always-present `behaviors` field is free until a unit actually has bindings. The `#[serde(default)]`
is required (old `obj:{id}` docs predate the field) and matches the one existing backward-compat
precedent, `Obj.powers`. One caveat: no `Obj` field currently uses `skip_serializing_if`, so a naive
add writes `"behaviors":{}` into every object doc — add
`#[serde(skip_serializing_if = "BTreeMap::is_empty")]` (a new convention for this codebase) to keep
empty maps out of the persisted JSON.

Raw `@set` or attribute editing must not attach behavior. Builders use lifecycle-aware commands:

```text
@behavior/add <object>=<behavior>
@behavior/remove <object>=<behavior>
@behavior/list <object>
```

A ruleset may provide friendlier wrappers, but every path reaches the same validated operation.
An optional `@xtype` compatibility command may translate a familiar type name into one or more
behavior bindings; a text attribute named `xtype` is never authoritative.

### 4.2 Registration

Extend `Registrar` with behavior registration. A registration includes:

- behavior id and owning module namespace;
- allowed `ObjKind` values;
- compatible and conflicting behavior ids;
- attach, detach, boot-validation, and destroy policies;
- contextual command specifications and handlers;
- optional help and inspection projection.

Only one module may own a behavior id. Binding an unknown behavior fails boot validation rather than
silently producing an inert object.

Lifecycle handlers receive a full `Ctx`, allowing object mutations, module persistence, output, and
deferred effects to remain in the current atomic command batch.

### 4.3 Contextual command dispatch

The existing `CommandFallback` is intentionally narrow, lacks a `Ctx`, and is used for script
`$`-commands. Do not overload it with typed simulation behavior. Add a behavior-command dispatch
stage owned by the core registry.

Initial dispatch order:

1. input contexts and their hard escape;
2. existing aliases and `HOME` handling;
3. exits;
4. global registered commands;
5. behavior commands on reachable command subjects;
6. existing script/module fallbacks;
7. `Huh?`.

This preserves current command and exit behavior. A behavior command cannot shadow a core verb,
staff command, or exit. TacSim verbs are nevertheless object-scoped: if no reachable object
provides `startup`, the command does not exist for that actor.

**Grounding:** the `CommandFallback` characterization is accurate — it is `&mut World` + `&mut Outbox`,
no `Ctx`, consulted only on an unresolved verb, used for script `$`-commands. The numbered list above
is a simplification of the real pipeline, which also has a single-char-prefix stage and a
`CommandGate` stage; map against `cmd.rs`, not this list. The behavior stage slots in at one clean
point — the `LookupErr::NotFound` arm, *before* the `CommandFallback` loop — which runs only after
exits and the command table have both missed, so "cannot shadow a core verb or exit" is structurally
guaranteed by that position, not by convention. Behavior command specs should be ordinary
`CommandSpec`s (carrying `scene_class`, permissions, switches) so turn-gating and the existing checks
apply uniformly (§6.2).

The dispatcher resolves candidate subjects in this order:

1. explicitly operated object published through a control assignment;
2. the actor object;
3. the actor's immediate MUSH container;
4. the actor's carried contents;
5. the containing room, when its command declares ambient reach.

The list is bounded and deduplicated. Finding the containing room follows only the object's bounded
location chain; dispatch never scans arbitrary nested contents or inheritance parents. An exact
match at a higher-priority tier wins. Multiple matches at the same tier produce an ambiguity error
naming the candidate objects rather than depending on registration order.

A `BehaviorCommandCtx` borrows the normal command context and adds the resolved subject:

```rust
pub struct BehaviorCommandCtx<'a, 'w> {
    pub ctx: &'a mut Ctx<'w>,
    pub subject: ObjId,       // object whose behavior supplied the command
    pub behavior: BehaviorId,
    pub reach: CommandReach,  // operated, self, location, carried, ambient
}
```

Structured switches, argument shapes, permission checks, help, deferred work, and persistence use
the same machinery as global commands.

### 4.4 Command-subject links

Some command subjects are not discoverable from ordinary containment. A pilot in a cockpit room
operates the containing vehicle; a rigger may remotely control a drone.

Infrastructure modules publish rebuildable command-subject links into `World`:

```rust
pub struct CommandSubjectLink {
    pub actor: ObjId,
    pub subject: ObjId,
    pub relation: SmolStr, // "pilot", "gunner", "remote", ...
    pub owner: SmolStr,    // module responsible for rehydration/removal
    pub priority: u16,
}
```

The authoritative assignment lives in `mush-sim`; the core index only accelerates dispatch. Links
are republished at boot and removed when an assignment, object, or behavior disappears.

---

## 5. Spatial model

### 5.1 The authoritative relationship

Every spatial entity has at most one authoritative spatial parent:

```rust
pub enum SpatialParent {
    Direct {
        domain: DomainId,
        pose: RulesValue,
    },
    Interior {
        space: ObjId,
        slot: Option<SlotId>,
    },
    Attached {
        host: ObjId,
        mount: MountId,
        local_pose: Option<RulesValue>,
    },
    Unplaced,
}

pub struct PlacementRecord {
    pub object: ObjId,
    pub parent: SpatialParent,
    pub revision: u64,
}
```

`RulesValue` is an opaque, schema-tagged value whose interpretation belongs to the domain driver:

```rust
pub struct RulesValue {
    pub schema: SmolStr,
    pub version: u32,
    pub data: serde_json::Value,
}
```

For example, a TacSim direct pose may contain hex coordinates, elevation, facing, and posture.
Another system may use metric coordinates, zones, lanes, or abstract range bands.

There is no independently authoritative `unit.map_id` plus `map.members` list. The placement graph
is truth; domain membership is a fast, rebuildable projection.

### 5.2 Invariants

`mush-sim` enforces these invariants on every transition and again during boot validation:

1. An object has no more than one spatial parent.
2. The parent graph is acyclic and has a configured maximum depth.
3. Every referenced object, domain, space, slot, host, and mount exists and has the required
   behavior.
4. A direct pose validates against its domain driver and current domain state.
5. Interior and mount capacity, size, mass, compatibility, and access policies validate before
   mutation.
6. A subtree resolves to at most one effective domain.
7. Ordinary `World::move_to` cannot silently contradict an active placement.
8. Structural transitions update placement, MUSH containment projection, command-subject indexes,
   derived domain indexes, and persistence in one world-task transaction.
9. Destroying or detaching a structural host cannot strand descendants.

### 5.3 Effective domain and presence

The effective domain of an object is resolved by walking its parents:

- `Direct` names the domain immediately.
- `Interior` follows the interior space's anchor.
- `Attached` follows the host.
- `Unplaced` has no effective domain.

An object's presence in that domain is separate from locality:

```rust
pub enum PresenceMode {
    Independent,
    DerivedExposed,
    Suppressed,
}
```

- **Independent:** the domain driver moves, collides, senses, and targets the object directly.
- **DerivedExposed:** the object inherits a host transform but remains visible, targetable, and able
  to act. Battle armor mounted externally on a mech is the initial example.
- **Suppressed:** the object is physically local to the domain but has no independent tactical
  projection. Units in hangars and infantry inside armored carriers are initial examples.

Presence is derived from the direct placement, interior compartment, or mount definition; callers
cannot set it independently to bypass exposure rules.

Maintain two indexes per domain:

- `local_objects`: all descendants whose effective locality is the domain;
- `projected_participants`: independent and derived-exposed participants the driver must consider.

Both indexes are rebuildable from durable records and are verified after every structural
transition in debug/test builds.

### 5.4 Interior spaces

An interior space is normally backed by a MUSH room so existing `look`, speech, exits, locks, and
contents work naturally:

```rust
pub enum SpaceAnchor {
    FixedDomain {
        domain: DomainId,
        anchor_pose: RulesValue,
    },
    MovingObject {
        host: ObjId,
    },
    Unanchored,
}

pub struct InteriorSpaceRecord {
    pub object: ObjId,
    pub anchor: SpaceAnchor,
    pub compartments: BTreeMap<SlotId, CompartmentSpec>,
    pub egresses: BTreeMap<EgressId, EgressSpec>,
}
```

A static hangar uses `FixedDomain`. A dropship bay uses `MovingObject`. Because RustMUSH rooms have
no ordinary `location`, this anchor is deliberately independent of the MUSH containment fields.

An interior may later host its own simulation domain. For example, a dropship can move on a world
map while an interior deck-plan domain runs a boarding engagement. That extension is not required
for the first vertical slice.

### 5.5 Attachments and mounts

Attachments are not disguised containment. A mount definition controls:

- allowed behavior/capability classes;
- capacity and exclusivity;
- local transform relative to the host;
- exposure and targetability;
- which actions remain available while mounted;
- detach conditions and egress pose;
- consequences when the host or mount is damaged or destroyed.

Initial use cases include battle armor riding a mech, externally carried units, towed units, and
equipment fixed to a platform.

### 5.6 Crew, passengers, and cargo

Physical location and operational role are separate relationships.

```rust
pub struct ControlStationRecord {
    pub station: StationId,
    pub host: ObjId,
    pub space: Option<ObjId>,
    pub role: SmolStr,
    pub capacity: u16,
    pub policy: RulesValue,
}

pub struct StationAssignment {
    pub station: StationId,
    pub actor: ObjId,
    pub assigned_at: i64,
}
```

A pilot is usually inside a cockpit and assigned to a pilot station. The station assignment grants
the mech as a command subject. Infantry transported in the same vehicle are inside a compartment
but have no control assignment. A remote-control assignment may grant a command subject without
physical containment.

Rulesets decide whether an actor may hold multiple stations, what happens linkdead, and how control
conflicts are resolved. The generic layer guarantees deterministic assignment and cleanup.

### 5.7 Relationship examples

| Situation | Spatial relationship | Presence | Other relationship |
|---|---|---|---|
| Mech on a world map | `Direct(domain, hex pose)` | Independent | Pilot station optional |
| Mech in a static hangar | `Interior(hangar room, bay)` | Suppressed | None |
| Mech in a dropship bay | `Interior(bay room, berth)` | Suppressed | None |
| Battle armor riding a mech | `Attached(mech, BA mount)` | Derived exposed | None |
| Infantry in an APC | `Interior(APC compartment)` | Suppressed | Cargo/passenger manifest |
| Pilot in a mech | Located in cockpit | Not a tactical unit | Pilot station assignment |
| Ejected pilot | `Direct(domain, pose)` | Independent | No station assignment |

---

## 6. Domains and clocks

### 6.1 Durable identity

A domain has a stable, persisted id independent of boot-scoped scheduler ids:

```rust
pub struct DomainId(pub u64);

pub struct DomainRecord {
    pub id: DomainId,
    pub host: ObjId,
    pub driver: SmolStr,       // e.g. "tactics.hex"
    pub schema_version: u32,
    pub status: DomainStatus,
    pub clock: DomainClock,
    pub revision: u64,
}
```

The host is the MUSH room or object carrying `sim.domain` plus a ruleset behavior such as
`tactics.map`. One host has one domain in the first implementation. Multiple domains may be hosted by
separate interior/map objects.

### 6.2 Clock modes

```rust
pub enum DomainClock {
    Passive,
    FixedStep { period_ms: u32 },
    TurnDriven,
}
```

- `Passive` domains retain placement but do not tick.
- `FixedStep` domains are used for real-time tactical maps.
- `TurnDriven` domains advance only when their bound scene or ruleset explicitly commits a turn.
  **A `TurnDriven` domain must ride the existing scene-turn mechanism, not a parallel turn engine.**
  The engine already turn-gates commands: a command self-declares `CommandSpec.scene_class`
  (`Free`/`Turn`), and `mush-scenes`' `TurnGate` refuses out-of-turn `Turn`-class commands in an
  active turn-based scene. A `TurnDriven` domain "commits a turn" through that same scene-turn
  advance, and any turn-sensitive behavior command (§4.3) — a mech's `FIRE` — carries
  `scene_class: Turn` so it gates identically to `+roll` or any other turn command. One turn
  concept, viewed at the command layer and the domain layer; never two implementations.

The generic scheduler records the next due instant; rulesets receive a fixed logical `dt`, domain
tick number, deterministic random source, current snapshot, and accepted intent set. Rules never
read wall time to resolve mechanics.

**Grounding:** the scheduler already exists. `mush-core`'s `Scheduler` is a min-heap keyed by
absolute deadlines whose `next_due()` is exactly "the next due instant," and the world task already
drives it via a `select!` over the command channel and a sleep-until-deadline. Periodic work already
flows through `reg.tick` with `Coarse`/`Fast`/`Scene` cadences (the tactics seam stub already ticks a
toy domain at 250 ms this way). Build `FixedStep` on `Scheduler`; do not invent a new timer. What is
genuinely net-new is the *durability* of scheduled state (the scheduler is in-RAM today, so pending
work is lost on restart) and the tick-commit/replay layer of §6.6 and §9.3 — not the scheduling.

### 6.3 Single-writer execution

There is no task or channel per unit or domain. Due domain ticks enter the existing single-writer
world loop as scheduled work. A tick may mutate only through its driver and the current `Ctx`.

Each fixed step is conceptually:

1. Drain the domain's accepted discrete intents for tick `N` and snapshot coalesced controls.
2. Build an immutable participant snapshot ordered by stable object id.
3. Resolve desired movement for all participants.
4. Resolve terrain, collisions, and simultaneous movement conflicts.
5. Commit positions and other continuous state together.
6. Rebuild spatial acceleration structures and LOS/contact projections.
7. Resolve actions, damage, heat, recycling, and other driver-defined phases.
8. Produce a semantic `TickCommit`, filtered observer deltas, metrics, and the next deadline.

The driver defines the exact phases, but it must document whether every phase reads the start-of-
phase snapshot or earlier committed phases. Iteration order must never create an undocumented game
advantage.

### 6.4 Lag policy and work budgets

- Never run an unbounded catch-up loop.
- `max_catch_up_steps` is configurable per driver, initially 4.
- Logical `dt` never expands to hide lag.
- If lag exceeds the catch-up budget, the domain enters `Lagged` and either skips idle wall time or
  pauses according to driver policy. The choice is recorded and announced to operators.
- Each tick records duration, participant count, intent count, output count, and persistence bytes.
- A driver has configurable maximum participants, intents per tick, and emitted events per tick.

### 6.5 Intent buffering and backpressure

Commands produce intents; they do not directly perform mid-tick movement or combat.

Two classes are required:

- **Continuous controls:** latest value wins per `(unit, control)`; heading and desired speed are
  initial examples. Repeated input replaces an existing value and cannot grow a queue.
- **Discrete actions:** ordered, assigned a sequence number, and bounded per unit and domain; firing,
  standing, mounting, and egress are examples.

Structural lifecycle transitions have their own validated path and are not shed as ordinary player
actions. When a discrete-action limit is reached, reject the new action explicitly. Do not silently
drop an accepted action and do not create another unbounded channel inside the world task.

### 6.6 Determinism

For identical durable state, driver version, tick number, random seed, and accepted intents, a tick
must produce the same commit and state hash on every supported platform. Use integer or explicitly
quantized arithmetic for authoritative state where floating-point differences could matter.

Every commit records:

- domain id and tick number;
- driver/schema version;
- accepted intent sequence range;
- deterministic RNG position or derived tick seed;
- structural and dynamic deltas;
- resulting authoritative state hash.

### 6.7 Driver registration and ownership

A domain driver is registered once during server assembly and is immutable after world load. The
driver id is globally unique. Conceptually it provides:

```rust
pub trait DomainDriver: Send + Sync {
    fn id(&self) -> &'static str;
    fn schema_version(&self) -> u32;
    fn validate_domain(&self, input: DomainValidation<'_>) -> Result<(), DriverError>;
    fn validate_pose(&self, input: PoseValidation<'_>) -> Result<(), DriverError>;
    fn validate_transition(&self, input: TransitionValidation<'_>)
        -> Result<(), DriverError>;
    fn step(&self, input: StepInput<'_>) -> Result<StepCommit, DriverError>;
    fn project(&self, input: ProjectionInput<'_>) -> Result<RulesValue, DriverError>;
}
```

`step` is a pure calculation over an immutable snapshot and accepted intents. It returns a proposed
commit; `mush-sim` validates and applies that commit through the world task. A driver cannot retain
`&mut World`, perform I/O, start tasks, or mutate another domain while stepping. Structural
cross-domain operations go through the transition service instead.

The initial assembly API should be a `SimulationModule` builder receiving `Arc<dyn DomainDriver>`
instances before `Engine::with_modules`. A ruleset supplies its driver and its ordinary
`GameModule` behavior registrations from the same crate. Driver registration must record the owning
module namespace so driver-owned snapshots, migrations, and persistence operations cannot escape
that namespace.

Phase C must settle the exact Rust ownership surface after a small compile-time prototype. Do not
introduce a global mutable driver registry or make ruleset modules reach into another module's
private state to work around registration.

---

## 7. Atomic transitions

All structural changes go through `mush-sim`; rulesets validate policy through the domain driver.

### 7.1 Entering a carrier or hangar

`embark(unit, space, slot, egress)`:

1. Confirm unit and destination share a reachable effective domain or approved connector.
2. Validate access, size, mass, capacity, state, and ruleset restrictions.
3. Remove the unit's independent/derived tactical projection.
4. Set its parent to `Interior`.
5. Move the MUSH object to the backing room when the space has one.
6. Update indexes and persist all affected records atomically.
7. Emit a semantic transition event only after the write batch succeeds.

### 7.2 Exiting

`egress(unit, space, egress)`:

1. Resolve the space anchor's current effective domain and transform.
2. Ask the driver to turn the egress definition into a candidate direct pose.
3. Validate occupancy, terrain, doors, velocity, and other restrictions.
4. Atomically replace `Interior` with `Direct` and update MUSH containment.
5. Add the unit's independent projection and emit the transition.

If the carrier moved or changed maps while the unit was inside, resolution naturally uses the
carrier's current domain and pose.

### 7.3 Attaching and detaching

`attach(child, host, mount)` and `detach(child, egress)` follow the same transaction pattern.
Detachment computes a direct pose from the host's current transform. If no legal pose exists, the
operation fails without partially removing the attachment.

### 7.4 Moving a carrier within or between domains

Movement within one domain changes only the carrier's direct pose. Descendant effective poses and
locality derive from the graph.

Cross-domain transfer occurs at a tick boundary:

1. validate the destination connector and the complete descendant subtree;
2. remove the root and projected descendants from the source indexes;
3. replace only the root's direct domain and pose;
4. add the root and projected descendants to destination indexes;
5. persist one atomic transfer commit;
6. notify both domains and bound scenes.

No descendant receives an independently authoritative copied `map_id`.

### 7.5 Ordinary MUSH movement

MUSH containment remains useful but is not tactical truth. `mush-sim` registers a `MoveAttempt`
gate:

- a directly placed or attached simulation participant cannot use ordinary exits or `@teleport` to
  bypass domain transitions;
- an interior occupant may use ordinary interior exits only when those exits do not cross a spatial
  boundary;
- egress portals invoke a spatial transition rather than raw `World::move_to`;
- staff recovery uses explicit simulation recovery commands and is audited.

The first implementation gives each domain and interior a configured MUSH presentation location.
Placement transitions mirror `Obj.location` when appropriate, but placement remains authoritative.

### 7.6 Destruction and forced recovery

Normal destruction is refused when the object owns a domain, anchors an interior, carries
descendants, hosts occupied stations, or has attached units. **Where that veto lives is a real
design decision, not a free hook — see D8.** Today there is no pre-destroy veto point at all:
`World::destroy` marks an object `GOING` immediately, and the only destroy seam is a *post*-destroy
observer (a cleanup net that cannot refuse). Adding a veto collides with a deliberate invariant —
`World` is observer-free; every `WorldEvent` is emitted by the command layer, never by `World`
itself — and `World::destroy` is also called recursively/programmatically (a room destroy cascades
into its exits), so a veto that lives only in the `@destroy` handler is silently bypassed by those
internal callers.

Staff receive explicit recovery choices such as evacuate, detach, unplace, or cascade. Forced
recovery must produce a complete plan before mutation and commit the plan atomically. Post-destroy
observers remain a final consistency net, not the normal lifecycle mechanism.

---

## 8. Generic unit and vehicle composition

Do not make `Mech` or `Vehicle` the foundational type. The reusable foundation is a simulation
participant plus optional capabilities expressed by behavior and ruleset data.

| Example | Generic capabilities | Ruleset-specific behavior |
|---|---|---|
| BattleMech | participant, controllable, mobile, crewed, sensors, weapons | heat, locations, actuators |
| Shadowrun car | participant, controllable, mobile, crewed, transport | handling, rigger, Matrix state |
| Drone | participant, controllable, mobile, sensors | autonomous/remote modes |
| Ship | participant, controllable, mobile, crewed, transport | compartments, power, flooding |
| Turret | participant, controllable, sensors, weapons | fixed facing/traverse |
| WoD vehicle | controllable, mobile, transport | chase and damage rules |

The generic layer should standardize only the capabilities needed for cross-system infrastructure:

- spatial participation and ancestry;
- control assignments;
- transport capacity and access;
- generic presentation metadata;
- domain intent submission.

Heat sinks, armor facings, initiative passes, hull sections, fuel, and similar rules remain opaque
driver state. Avoid turning the generic layer into a large entity-component taxonomy before two
rulesets demonstrate the same requirement.

### 8.1 Occupants, controllers, and control leases

Physical occupancy, station assignment, and present control authority are separate facts. A human
pilot may remain physically seated while onboard or remote automation temporarily controls the
unit. A named NPC wingman is an actor assigned to a station; an emergency autopilot is not required
to invent another person or occupy a physical seat.

```rust
pub enum ControllerRef {
    Actor(ObjId),
    Automation { behavior: BehaviorId, instance: SmolStr },
    Remote(ObjId),
}

pub struct ControlLease {
    pub unit: ObjId,
    pub station: StationId,
    pub controller: ControllerRef,
    pub epoch: u64,
    pub acquired_tick: u64,
    pub reason: ControlLeaseReason,
}
```

A control channel has at most one primary lease. Multi-crew units may have independent channels
such as movement, weapons, sensors, and command, with ruleset-defined arbitration between them.
Every submitted intent carries the lease epoch; revocation invalidates queued or late intents from
the former controller.

### 8.2 and beyond — automated control, tactical AI, and mobility (moved)

Sections 8.2 through 8.13 of the original draft — link-loss fallback and connection lifecycle,
reclaiming control, anti-exploit invariants, allegiance/disposition/command authority, typed
tactical orders, hybrid tactical AI, bounded LLM latency, tactical knowledge and memory, named
pilots and group controllers, `mush-npc` scheduling, and NPC-driven mobility services — now live
in the companion document, **`docs/design/07-npc-control-and-tactical-ai.md`**. They describe who
or what controls a spatial unit when a human is not present, and how that controller reasons. That
layer sits entirely downstream of this foundation and its toy driver, and is not required for
Phases A–E here.

---

## 9. Persistence and recovery

### 9.1 Generic records

Initial Redis documents in the `sim` namespace:

```text
mod:sim:meta
mod:sim:domain:<domain-id>
mod:sim:placement:<obj-id>
mod:sim:space:<obj-id>
mod:sim:station:<station-id>
mod:sim:assignment:<station-id>:<actor-id>
mod:sim:domain:<domain-id>:checkpoint
mod:sim:domain:<domain-id>:commits
```

Ruleset data stays under its own namespace:

```text
mod:tactics:map:<domain-id>
mod:tactics:unit:<obj-id>
mod:tactics:domain:<domain-id>:runtime
```

Exact Redis key spelling may change during implementation, but identity, ownership, and atomicity
must not.

### 9.2 Structural writes

Behavior binding changes, placement transitions, MUSH containment changes, and affected ruleset
state commit in the same world-task `WriteBatch`. Output describing success is held until that batch
is acknowledged by the store, following existing server policy.

### 9.3 Tick commits

Do not persist one full document per unit per fast tick. A domain tick produces one compact,
ordered commit containing its deltas and state hash. Periodic checkpoints contain the complete
driver state needed to resume. Recovery loads the newest checkpoint and replays later commits.

Use a dedicated Redis sorted set for this bounded recovery log, not the existing
`ModuleWrite::StreamAppend` ring-history operation. The score is the domain tick and the member is a
canonical, versioned commit envelope containing that tick and its state hash. Retrying `ZADD` with
the same member is idempotent; recovery treats two different members at the same tick as corruption
rather than guessing an order.

After producing a checkpoint through tick `N`, commit the checkpoint `JSON.SET` and
`ZREMRANGEBYSCORE commits -inf N` in the same `WriteBatch`. Thus a commit is removed only in the
transaction which makes an equal-or-newer recovery point durable. Do not apply `MAXLEN` or any
independent retention cap to this log.

Critical structural transitions are never delayed until a periodic checkpoint.

### 9.4 Boot recovery

At boot:

1. Load behavior-bound objects and all `sim` records.
2. Validate references, behavior ownership, graph acyclicity, and schema versions.
3. Load each driver's durable data and migrate supported versions.
4. Restore domain checkpoints and replay commits, verifying state hashes.
5. Rebuild locality, projected-participant, station, and command-subject indexes.
6. Reconcile MUSH containment projections without overwriting valid player data silently.
7. Restore active clocks in `PausedAfterRestart` unless a driver explicitly declares safe automatic
   resumption.
8. Publish diagnostics and require staff recovery for quarantined records.

A partially understood domain never resumes ticking.

---

## 10. Presentation and information security

Ruleset logic emits semantic views, not terminal escape sequences or HTML.

This is the layer that will make or break the tactical experience, and it deserves its own sustained
effort. The projections a TacSim engagement actually needs are richer than a single map frame:

- **unit status** — the operated unit's heat, movement, weapon-by-weapon status, ammunition, and
  armor/internals by location, as a live self-updating panel;
- **dual-scale tactical display** — a wide strategic view (roughly an N-hex radius centered on the
  operated unit, so the pilot reads the shape of the battle) *and* a zoomed navigation view for
  immediate hex-by-hex movement around obstacles and terrain;
- **contact changes** — new/updated/lost contacts as *perceived*, never as truth (see the security
  rule below);
- **damage report output** — incoming and outgoing hit resolution as readable events;
- **team comms** — the tactical channel(s) for the operated unit's side or formation;
- **tick/event notifications.**

These are auto-updating panels, not one-shot renders: a keyframe followed by deltas, each carrying
projection revision and domain tick so a client can request a fresh keyframe after a gap.

The server must never send raw map truth and rely on the client to hide it. The domain driver builds
an observer-specific projection after LOS, sensors, permissions, and fog-of-war checks — and that
projection is the tactical face of the shared epistemic substrate (companion doc §6), not a separate
fog-of-war. `TACTICAL` asks the operated unit behavior for its observer projection; `VIEW <x> <y>`
is supplied by a reachable map/domain behavior and is permission-gated. The renderer does not perform
game rules.

**Grounding — this is a major, mostly-unbuilt workstream, not a thin projection.** The one-protocol
`OutputEvent`/`Panel` design is real (one semantic event, multiple renderers), and a `tactics.map`
`Panel` feed already exists in the seam stub. But today a *generic* module `Panel` feed renders on
**web only**: the SSH line renderer drops panel frames silently, the SSH TUI renders only a few
hardcoded built-in panels (mail/intel/who) via query replies, and the promised text-fallback
registrar seam is unbuilt. So "SSH line ASCII map + TUI cell grid + web canvas, all from one
visibility-filtered projection" is a substantial build in itself: a panel→line ASCII renderer, a
panel→TUI grid renderer, the panel-renderer registration seam, and the dual-scale projection logic.
Budget for it as a first-class workstream and sequence it explicitly — a tactical sim that can only
be seen on the web transport is half a feature.

---

## 11. Scenes and event journaling

A durable narrative scene may bind to a domain:

```rust
pub struct SceneDriverBinding {
    pub provider: SmolStr, // "sim"
    pub kind: SmolStr,     // "domain"
    pub id: SmolStr,       // durable DomainId representation
}
```

This reference is deliberately generic so `mush-scenes` does not depend on `mush-sim` or a
ruleset. A core-registered scene-journal service accepts semantic mechanical events from a bound
driver. Events include domain tick and stable object ids but contain no rendered text.

Binding policy:

- a domain can exist with no active scene;
- a scene can bind to at most one domain initially;
- a domain may have at most one exclusive active engagement scene initially;
- scene participants and tactical projections are separate sets;
- crew and contained passengers may be scene participants without being tactical targets;
- ending a scene does not delete the map or placements;
- pausing an exclusive domain-bound scene pauses its domain clock after the current atomic tick.

Timestop continues to use `SceneMode::TurnBased` — and a `TurnDriven` domain bound to a turn-based
scene advances through that scene's existing turn machinery, not a second one (§6.2). A future
real-time scene mode describes the narrative clock while the bound `FixedStep` domain retains
authority for fixed-step mechanics; pausing the scene pauses the domain clock after the current
atomic tick (above).

---

## 12. Permissions, administration, and observability

### 12.1 Permissions

- Attaching/removing behavior requires `controls()` plus the owning module's builder permission.
- Creating a domain or changing its driver requires staff or a dedicated power.
- Operating a unit requires a valid control assignment, remote-control grant, or staff override.
- Embark, egress, attach, and detach require access policy and ruleset validation.
- Direct placement edits are staff recovery operations and are audited.
- Scripts may request public transition APIs but may not mutate placement records or indexes.

### 12.2 Staff commands

Proposed generic administration surface:

```text
@behavior/list <object>
@behavior/add <object>=<id>
@behavior/remove <object>=<id>

@domain/create <host>=<driver>
@domain/status <domain-or-host>
@domain/pause <domain-or-host>
@domain/resume <domain-or-host>
@domain/validate <domain-or-host>

@spatial/show <object>
@spatial/tree <object>
@spatial/place <object>=<domain>,<ruleset pose>
@spatial/unplace <object>
@spatial/recover <object>=<policy>
```

Ruleset builder commands create maps, unit templates, bays, mounts, and stations; generic commands
inspect and recover infrastructure.

### 12.3 Metrics

Expose at least:

- active, passive, paused, and lagged domain counts;
- tick duration histogram by driver;
- tick lag and skipped/paused catch-up events;
- projected and local participant counts;
- accepted, coalesced, rejected, and executed intents;
- LOS/contact computation time;
- tick commit and checkpoint bytes;
- graph depth and validation failures;
- behavior-command matches and ambiguities.

Add domain clocks and intent depths to `@ps` without dumping hidden tactical data.

---

## 13. Failure handling

| Failure | Required behavior |
|---|---|
| Invalid behavior binding on boot | Quarantine binding; do not invoke it; report exact object/id |
| Placement cycle or missing parent | Pause affected domains; quarantine subtree; require recovery |
| Driver panic/error | Abort tick commit, pause domain, retain last durable state |
| Persistence unavailable | Follow global mutation-pause policy; do not advance authoritative ticks |
| Client too slow | Drop/rebuild presentation deltas per transport policy; never stall domain |
| Intent limit exceeded | Reject new discrete intent explicitly; continuous control remains coalesced |
| Tick exceeds budget | Record lag and apply bounded catch-up policy |
| Carrier transfer validation fails | Leave complete subtree in source domain unchanged |
| Egress pose blocked | Leave unit contained/attached and report reason |
| Host destroy requested with descendants | Veto normal destroy and show recovery choices |

---

## 14. Testing strategy

### 14.1 Generic unit and property tests

- Behavior ids, compatibility, attach/detach lifecycle, and unknown bindings.
- Contextual command precedence, ambiguity, permission, and cleanup.
- At most one spatial parent per object.
- Cycle rejection at every graph depth and boot-time corruption detection.
- Effective-domain and presence derivation.
- Rebuildable indexes equal a full graph traversal after randomized transitions.
- Capacity, slot, and mount validation.
- Atomic failure leaves placement, MUSH location, indexes, and persistence ops unchanged.
- Continuous controls remain O(units x control kinds), not O(input lines).
- Discrete intent bounds and deterministic ordering.
- Tick replay produces the recorded state hash.

Use property tests for randomized spatial forests, embark/egress/attach/detach sequences, and
checkpoint/replay boundaries.

### 14.2 Acceptance scenarios

1. Create a map object and attach domain/map behavior.
2. Create a mech object in a static hangar and attach unit/mech behavior.
3. Assign a pilot station; verify only the pilot sees unit commands.
4. Egress onto the world map at the hangar portal.
5. Start the unit, set heading and desired speed, tick deterministically, and render `TACTICAL`.
6. Enter a dropship bay on the same map; verify the mech loses independent presence.
7. Move the dropship; verify the contained mech follows without a rewritten direct pose.
8. Transfer the dropship to another map and egress the mech at the new position.
9. Attach battle armor; verify inherited pose plus exposed targetable presence.
10. Embark infantry as cargo; verify locality without independent tactical presence or vehicle
    controls.
11. Restart from Redis; verify graph, assignments, domain state, and paused clock.
12. Run the same seed and intent trace twice and compare every tick state hash.
13. Compare SSH ASCII, TUI, and web projections for semantic equivalence and absence of hidden
    contacts.
Scenarios 14 through 25 (link-loss and reclaim, allegiance and orders, bounded-LLM timing, tactical
belief, group control, and mobility) move to the companion document's testing section
(`docs/design/07-npc-control-and-tactical-ai.md`, §10), since they exercise the control-automation
and tactical-AI layer rather than this foundation.

The existing `mush-tactics` seam tests remain green throughout. Once the real domain APIs cover
their purpose, rewrite those tests against the new generic surface rather than deleting the standing
coverage.

---

## 15. Provenance and game-content boundary

Implementation work under this specification must be independently written from RustMUSH
requirements and the public, player-visible behavior the project chooses to support.

- Do not translate or port third-party source, comments, layouts, serialization formats, lookup
  tables, formulas, or tests.
- Do not use matching internal names merely because another implementation used them. Familiar
  player-facing verbs may be treated as UX requirements only after project review.
- Write expected behavior and tests from this specification before implementing each subsystem.
- Record the origin and license of imported map, unit, rules, art, and text data separately from the
  server-code design.
- Treat permission to implement or distribute any published system's game content, trademarks, and rules data (TacSim is our own generic ruleset)
  as a separate question from permission to use any existing server source.

If a formally clean-room implementation is required, source-exposed specification authors and
unexposed implementers must be separated under a process reviewed by qualified counsel. This draft
documents engineering boundaries, not a legal conclusion.

---

## 16. Implementation phases

### Phase A: behavior foundation

- Add `BehaviorId`/`BehaviorBinding` to `Obj` with migration/defaults.
- Add registrar ownership, lifecycle hooks, validation, and staff commands.
- Add bounded contextual command-subject links and behavior-command dispatch.
- Prove with a toy `vehicle.console` behavior in `mush-testkit`.

**Exit criterion:** two objects can carry the same behavior and expose the same contextual verb;
an unbound object does not expose it; persistence and ambiguity tests pass.

### Phase B: spatial graph without ticking

- Add `mush-sim` records, persistence, boot validation, indexes, and inspection.
- Implement direct, interior, attached, and unplaced relationships.
- Implement stations/assignments and command-subject publication.
- Gate ordinary MUSH movement and destruction.

**Exit criterion:** the hangar, dropship transfer, battle-armor attachment, infantry cargo, and pilot
assignment scenarios work without a ruleset-specific tick.

### Phase C: deterministic domain runner

- Add durable domain clocks and scheduled domain ticks.
- Add bounded/coalesced intents, fixed-step execution, lag policy, state hashing, commits, and
  checkpoints.
- Build a toy driver using integer grid movement.

**Exit criterion:** deterministic trace/replay and restart tests pass under load with bounded memory.

### Phase D: TacSim movement slice

- Replace the toy TacSim stub state with `tactics.map`, `tactics.unit`, and `tactics.unit` behavior.
- Add a hex pose/map schema, startup state, heading, desired/current speed, and movement.
- Add `STARTUP`, `SHUTDOWN`, `HEADING`, `SPEED`, `TACTICAL`, and map `VIEW` as behavior commands.
- Add control leases (§8.1). The deterministic link-loss fallback controller that rides those leases
  is companion-doc work and is not required for this slice.
- Emit visibility-filtered `tactics.map` keyframes and deltas.

**Exit criterion:** the first five acceptance scenarios work through SSH line mode and one structured
panel client. (The link-loss/reclaim scenarios are in the companion document, gated on its
automation phases.)

### Phase E: transport and nested locality

- Add TacSim bay, mount, embark/egress, carrier transfer, battle armor, infantry cargo, and
  destruction policies.
- Add cross-domain connectors and subtree transfer commits.

**Exit criterion:** acceptance scenarios 6 through 10 pass, including restart at every transition
boundary.

### Phases F–G: tactical interaction, scenes, and NPC forces (companion)

The tactical-interaction, engagement-scene, and NPC-force phases move to the companion document
(`docs/design/07-npc-control-and-tactical-ai.md`, §11): terrain/LOS/sensors/weapons/damage, typed
missions and orders, controller belief states, real-time engagement scenes bound to domains, named
NPC pilots, salience activation, and the optional flash-model route. They follow Phases A–E here.

**Sequencing recommendation.** Build Phases A and B against the toy `vehicle.console`/integer-grid
driver first — that is the reusable spine, and it is mostly grounded in existing machinery (the
`Scheduler`, tick cadences, the `MoveAttempt` veto, the transient scene-state/map-blob store). Then
pull the **mobility / route-interception** case (companion §9) *forward* as the second cross-ruleset
proving ground, ahead of the full TacSim combat slice: it is the "no free teleport, routes are
legible" payoff, it is genuinely cross-system (dropships and Uber-for-WoD alike), and it exercises
coarse legs, connectors, fog-of-war, and promotion-to-live without needing a combat stack. Only then
harden Phase C's full fixed-step determinism/replay and build Phase D's TacSim hex slice, with
the presentation workstream (§10) running alongside D rather than after it.

---

## 17. Decisions and rationale

### Established design decisions

- Object behavior is a first-class binding, not a magic attribute or marker.
- Behavior commands are contextual and use a full `Ctx`.
- The authoritative spatial model is a single-parent acyclic graph.
- Domain membership and effective map position are derived, rebuildable projections.
- Containment, attachment, control assignment, and scene membership are distinct relationships.
- Station occupancy and present control authority are distinct; all intents carry a control epoch.
- Control authority, allegiance, tactical AI, and belief-state perception are elaborated in the
  companion document — its load-bearing principles: link loss hands control to deterministic
  fallback automation and never removes a unit; faction, operation side, observer-relative
  disposition, and command authority are separate; tactical AI is deterministic-first with the LLM
  as a bounded, optional deliberator that never owns ticks; and controllers receive per-observer
  belief state (a projection of the shared epistemic substrate), never hidden map truth.
- Maps/domains have durable identities and own deterministic ticking.
- Narrative scenes and simulation domains remain separate but bindable.
- Real-time state uses compact tick commits plus checkpoints, not per-unit full writes.
- TacSim is the first driver, not the generic data model.

The following decision records (D1–D8) resolve the foundation's open questions. Their recommended
choices are the normative defaults for this draft; deployments may tune values only where a decision
explicitly defines a configuration surface.

### D1. MUSH location projection for domain participants

| Option | Advantages | Problems |
|---|---|---|
| Use the domain host object directly | fewest records | a domain may be hosted by a Thing that is not the desired social room |
| Let each driver choose arbitrary rooms per unit | supports sectors immediately | fragments locality and gives rulesets control of core containment invariants |
| Give each domain one explicit presentation location | separates host from presentation while preserving one generic invariant | adds one validated reference |

**Decision:** add `DomainRecord.presentation_location: ObjId`. Every directly placed participant
projects to that location; a visible attached participant projects to the same location as its
effective parent. Interior occupants retain ordinary containment under their interior's object.
The presentation location may equal the domain host but need not.

`Obj.location` is always a transactionally maintained presentation projection, never the source of
tactical position. A driver cannot choose an arbitrary room per unit. If a later ruleset proves it
needs geographical sectors, add a generic, deterministic `PresentationResolver` after the
single-location model works; do not encode sectors as driver side effects.

### D2. Domain identity allocation

| Option | Advantages | Problems |
|---|---|---|
| Reuse the host `ObjId` | familiar external id | deleting/recreating a domain on the same host makes stale references ambiguous |
| Use `(host, generation)` | detects host reuse | compound identifiers leak lifecycle mechanics through every API |
| Allocate an independent monotonic id | stable history and simple references | requires a small allocator record |

**Decision:** `mush-sim` allocates a never-reused `DomainId(u64)` from `mod:sim:meta`, in the same
atomic batch which creates the domain. A separate uniqueness constraint permits at most one active
domain on a host object initially. User-facing output may show both `domain 42` and `host #123`.

This keeps scene bindings, journal provenance, connector endpoints, and old diagnostics unambiguous
when behavior is removed and later reattached to the same MUSH object. No generation component is
needed because `DomainId` itself is never recycled.

### D3. Generic capacity and load representation

| Option | Advantages | Problems |
|---|---|---|
| Require SI mass and volume | easy cross-unit comparison | many rulesets use slots, size classes, or abstract cargo and would invent false precision |
| Keep all capacity opaque to each ruleset | maximum freedom | generic containment cannot preflight or explain a transfer |
| Use namespaced resource vectors plus compatibility tags | supports common validation without pretending all games share physics | rulesets must register their measures |

**Decision:** the generic layer owns exact integer resource accounting and tag matching, while the
ruleset defines the dimensions and any complex policy:

```rust
pub struct LoadProfile {
    pub tags: BTreeSet<LoadTag>,
    pub measures: BTreeMap<MeasureId, i64>,
}

pub struct CapacitySpec {
    pub accepts: TagPredicate,
    pub limits: BTreeMap<MeasureId, i64>,
    pub maximum_items: Option<u32>,
}
```

Measures are namespaced and registered with a fixed scale, for example `core.count`,
`si.mass_g`, `tactics.bay_slots`, or `shadowrun.seats`. The generic layer only sums and compares
matching measure IDs and never uses floating point. Physical SI measures are encouraged when the
ruleset genuinely has them, not required. Irregular constraints such as BattleMech repair state,
door geometry, or a rigger interface remain driver validators which run before the generic atomic
transition.

### D4. Forced-destroy recovery operations

| Option | Advantages | Problems |
|---|---|---|
| Implicit cascade | convenient | catastrophic typo surface and poor recovery/audit semantics |
| Always move descendants to Limbo | avoids deletion | silently destroys spatial meaning and can violate ruleset invariants |
| Veto, then require a previewed recovery plan | explicit and auditable | costs an extra staff step |

**Decision:** normal `@destroy` is vetoed while structural dependents exist. Expose these staff
recovery operations behind a dry-run planner:

1. **Evacuate** a subtree to a specified compatible interior, domain connector, or static room,
   only when every resulting placement validates.
2. **Detach** attached descendants to driver-computed legal poses, only when all poses can be
   reserved atomically.
3. **Quarantine** a subtree as `Unplaced`, project it to a configured staff recovery room, and keep
   a durable recovery record. This is the last-resort non-destructive operation.
4. **Cascade destroy** only as a disabled-by-default God operation requiring the exact preview
   token and object-graph revision. Never accept a wildcard or silently fall back to cascade.

The preview lists every affected object, controller, scene, domain, connector, and persistence
record and returns a short-lived plan token. Apply revalidates the graph revision and commits the
whole plan atomically. A domain host must be paused before recovery. Post-destroy observers remain
consistency alarms, not an implementation path.

### D5. Checkpoint-aware recovery log

The recovery log records domain-tick commits so a crash can replay from the last checkpoint. It
needs three properties: idempotent append (retries must not duplicate), removal of committed ticks
at a checkpoint boundary (it must stay bounded), and tick-range reads for replay. This is a
*different shape* from `mush_core::Ledger` — the authoritative, append-only, never-trimmed log that
canon and scenes already use — so the Ledger cannot serve it, and that is expected.

| Option | Advantages | Problems |
|---|---|---|
| Existing `StreamAppend` with `MAXLEN ~` | already implemented | trims past the checkpoint and `XADD *` duplicates on an ambiguous retry |
| Dedicated sorted-set (`ZADD` + `ZREMRANGEBYSCORE`) | native trim + score-range reads | two new store write ops across every exhaustive `ModuleWrite` match, plus a net-new range-read trait method in both store impls and the conformance suite |
| Rewritten `{tick → commit-envelope}` JSON doc | idempotent whole-doc `JsonSet`, trivial in-RAM range/replay, prunes below the checkpoint before rewrite, **zero new store primitives** | whole-doc rewrite cost scales with the retained window × commit cadence |

**Decision:** default to the **rewritten JSON-doc** log — one `{tick → versioned-commit-envelope}`
document per domain, rewritten whole on each commit (idempotent on retry — exactly the
`mush_core::Ledger` trick and the codebase's dominant persistence pattern), with ticks at or below
the checkpoint pruned from the map before the rewrite. This needs no new store operation: it rides
plain `JsonSet` + `Del`, commits in the existing `MULTI`/`EXEC` `WriteBatch` alongside the
checkpoint, and replays over an in-RAM map. `StreamAppend` is rejected for the same reasons the
Ledger already documents (its `XADD *` retries double-append and it trims blindly) — cite that
precedent rather than re-deriving it.

Reserve the dedicated sorted-set (`ZADD` + `ZREMRANGEBYSCORE` + a bounded `ZRANGE ... BYSCORE`) for a
*proven* hot path — a large retained window or a high commit cadence where rewriting a growing
whole-doc becomes the bottleneck. If added, it is two new write ops (edited into every exhaustive
`ModuleWrite` match) plus a range-read trait method in the Redis and in-memory stores and the
conformance suite, and domain-tick scores must stay within Redis's exact-integer (`2^53 - 1`) range.
Either way the checkpoint write and the commit prune land in one `WriteBatch`, so a commit is
discarded only in the transaction that makes an equal-or-newer recovery point durable.

### D6. Restart behavior

| Option | Advantages | Problems |
|---|---|---|
| Pause every domain | safest and easiest to inspect | persistent background worlds require staff action after every restart |
| Resume every domain | seamless | can resume corrupted, obsolete, or combat-critical state without observation |
| Per-domain policy with a conservative default | supports both use cases | needs explicit readiness gates |

**Decision:** default to `PauseOnRestart`. Permit an operator to select `AutoResume` only when all
of the following are true:

- the driver version declares restart-safe replay;
- checkpoint and commit replay complete with matching hashes;
- graph/index reconciliation finds no quarantine or unresolved transfer;
- the domain has an explicit downtime policy and an operator-enabled auto-resume flag.

Auto-resume enters a short `ReadyAfterRestart` warm-up, republishes a keyframe and announcement,
then advances from the next logical tick. It does not catch up one fixed step per second of wall
clock downtime. Any offline economic advancement is a separate, bounded driver operation, never an
accidental fast-forward of combat. Player-controlled and active engagement domains should retain
the pause default even if passive world traffic is later opted in.

### D7. Scene-journal integration seam

| Option | Advantages | Problems |
|---|---|---|
| `mush-sim` calls `mush-scenes` directly | straightforward | creates the dependency the generic driver binding was designed to avoid |
| Publish untyped JSON on a general event bus | loosely coupled | weak ownership, validation, delivery, and replay guarantees |
| Register one narrow typed core service | decoupled and testable | requires a small shared envelope in `mush-core` |

**Decision:** define `MechanicalJournalEvent` and a `SceneJournalSink` registration seam in
`mush-core`. `mush-scenes` implements and registers the sink; `mush-sim` and future drivers only
know the core contract.

```rust
pub struct MechanicalJournalEvent {
    pub source: SceneDriverBinding,
    pub source_sequence: u64,
    pub logical_time: LogicalTime,
    pub actor: Option<ObjId>,
    pub witnesses: Vec<ObjId>,
    pub kind: SmolStr,
    pub payload_version: u16,
    pub payload: serde_json::Value,
}
```

The envelope is typed; `kind` and versioned payload remain provider-owned. The sink resolves active
scene bindings, appends one semantic event, and deduplicates by `(source, source_sequence)`. It
adds persistence to the same `WriteBatch` as the mechanical commit. Witnesses and visibility are
captured from the driver's perception result at event time rather than recomputed from later world
state. With no bound scene the sink is a no-op; creating a required binding fails if no sink is
registered.

### D8. Destroy-attempt veto placement

| Option | Advantages | Problems |
|---|---|---|
| Hook inside `World::destroy` before `GOING` | catches every destroy path, including recursive/programmatic ones | breaks the deliberate observer-free-`World` invariant (all `WorldEvent`s come from the command layer) |
| Veto only in the `@destroy` command handler | respects observer-free `World`; mirrors the existing `controls`/`SAFE` checks | bypassed by internal callers of `World::destroy` (a room destroy cascading into its exits) |
| Command-layer veto + validate-before-mutate transition service + post-destroy backstop | respects the invariant and closes the internal-caller gap without a `World` hook | the transition service must own structural-dependency validation, not `World` |

**Decision:** the third option. Structural-dependency checking — does this object own a domain,
anchor an interior, carry descendants, host occupied stations, or have attached units — lives in the
`mush-sim` transition service and is consulted by the `@destroy`/recovery command layer *before* any
mutation, the same validate-then-apply discipline every transition uses (a handler `Err` after a
world mutation does not roll back RAM). `World::destroy` stays observer-free. The internal
recursive/programmatic paths are covered because the veto refuses at the top — destroying a domain
host is refused before its exit cascade can begin — and the post-destroy `ObjDestroyed` observer
remains the consistency backstop, never the primary mechanism (§7.6). This adds no veto hook inside
`World` (which nothing else needs) while guaranteeing no destroy path silently strands descendants.

### Link-loss decisions (moved)

The two link-loss decisions from the combined draft — **timing defaults** and **ownership of policy
choices** — move with the automation layer to the companion document
(`docs/design/07-npc-control-and-tactical-ai.md`, §12, as its D1 and D2). They belong with the
control-automation model, not the spatial foundation.

These decisions (D1–D8) close the architectural questions required for Phases A–C. Exact tuning
constants such as checkpoint interval and warm-up duration should be selected by deterministic load
tests and production telemetry, without reopening the boundaries above. The tactical and automation
decisions live in the companion document.
