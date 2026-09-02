# NPC control and tactical AI in the spatial model

**Status:** Draft 0.1 for design review
**Scope:** automated unit control, link-loss fallback, allegiance and typed orders, hybrid
deterministic+LLM tactical AI, per-controller belief state, named NPC pilots and group controllers,
and NPC-driven mobility services
**Companion to:** `docs/design/06-object-behaviors-spatial-domains.md`

This document was originally the back half of doc 06. It is split out because it answers a distinct
question — *who or what controls a spatial unit when it is not a present human, and how does that
controller reason* — and that question must not gate the spatial foundation. Doc 06 stands on its
own; this one assumes it: domains, the placement graph, **control leases** (doc 06 §8.1), atomic
transitions, the domain driver, the perception/projection seam, and the scene-journal seam.

Everything here is downstream of a working doc-06 foundation and a toy domain driver. None of it
should be implemented before doc 06's Phases A–C are green, and much of it (the tactical AI,
orders, and mobility) should follow the first real ruleset driver. Treat the decision records here
as *recommended defaults for when we build this*, not as commitments that bind the foundation.

---

## 1. Control automation and link-loss

A spatial unit (doc 06 §8.1) has a **control lease**: at most one primary lease per control
channel, each intent stamped with the lease epoch, revocation invalidating queued or late intents.
This section is about what holds a lease when a human is not present, and how a unit behaves through
the seam of a player leaving and returning.

### 1.1 Link-loss fallback policy

Losing the last live player connection never removes, teleports, protects, or unplaces a unit. It
starts a control-interruption episode and applies a predeclared fallback policy:

```rust
pub struct LinkLossPolicy {
    pub takeover_after_ms: u32,
    pub objective: LinkLossObjective,
    pub engagement: LinkLossEngagement,
    pub reclaim: ReclaimPolicy,
    pub maximum_duration_ticks: Option<u64>,
}

pub enum LinkLossObjective {
    ContinueCurrentPlan,
    ContinueToDestination,
    ReturnToSortieOrigin,
    SeekNearestFriendlyRecovery,
    HoldPosition,
}

pub enum LinkLossEngagement {
    EvadeAtMaximumSpeed,
    FightingRetreat,
    DefendSelf,
    StandAndFight,
}
```

The sortie record supplies origin, destination, approved connectors, and fallback recovery sites.
Returning home is ordinary simulated travel through the same domains and hazards, never a recall
or despawn operation. If the preferred objective becomes impossible, the ruleset follows a
predeclared fallback chain and reports the reason.

Emergency control must not depend on an LLM call. A deterministic executor immediately maintains
safe motion and the current plan; at the takeover deadline a deterministic utility controller
begins the selected fallback. An optional LLM deliberator may later refine that plan, subject to the
normal deadline, staleness, and legality checks.

### 1.2 Connection lifecycle integration

**Grounding note (corrects the combined draft).** An earlier draft assumed the engine only exposed a
*per-connection* `Disconnected` and that session-level zero↔nonzero tracking had to be built. That
is wrong, and the design is cheaper as a result:

- `WorldEvent::Connected`/`Disconnected` are **already session-level zero↔nonzero transitions**, not
  per-connection. `Connected` fires only on the true 0→1 connect; a 2nd/3rd connection of a
  multi-connected character fires no world event. `Disconnected` fires on a clean last-connection
  `QUIT`/`@boot`, or on linkdead-grace expiry.
- The engine already tracks live connections independently of social presence:
  `World::is_connected` stays true through linkdead while live connections are zero. The 300-second
  linkdead grace already exists (`mush-session` `linkdead_grace`), and the session layer already
  models the exact transitions this system needs — `AttachOutcome::Rejoined { from_linkdead: true }`
  (first connection back out of linkdead) and `DetachOutcome::Linkdead { grace }` (last connection
  lost) with `DetachReason::is_clean()`.

So the RustMUSH session/connection machinery is not the thing to rebuild. What is **genuinely
net-new**, and all this system needs, is:

1. Two **observable world events** the sim layer can subscribe to, which today exist only as room
   text and session-layer outcomes: **linkdead-onset** (last live connection lost, unclean, before
   grace) and **linkdead-revival** (first connection returned out of linkdead). Each carries a
   monotonically increasing availability generation and the clean/lost reason.
2. A **separate, shorter simulation-control grace** hung off linkdead-onset, decoupled from the
   300 s session grace. The session grace governs whether the *login session* survives; the sim
   grace governs whether an *uncontrolled tactical unit* is allowed to coast. The latter is much
   shorter (see §12 D1) and is scheduled on the existing due-time scheduler at a domain tick
   boundary.

On linkdead-onset:

1. Keep the human actor in the station and the unit in its domain.
2. Start or continue one control-interruption episode.
3. Schedule takeover at a domain tick boundary using the availability generation and lease epoch.
4. Continue the last safe controls during the short grace period.
5. At the deadline, revalidate that no connection returned and atomically grant the fallback lease.
6. Journal the loss, policy, controller, and resulting plan.

Repeated connection flapping does not restart the episode indefinitely. A connection must remain
stable for a configured interval before a later loss receives a fresh grace period. A clean `QUIT`
may use immediate takeover while an unclean loss receives the short sim-grace allowance; the domain
policy decides, but neither path removes the unit.

The existing `Disconnected` world event (clean quit or grace expiry) remains appropriate for social
presence and ordinary NPC stimuli. Simulation handoff must use the new **linkdead-onset** event
rather than waiting for `Disconnected`, because the whole point is to react while the character is
still `is_connected` for WHO but has zero live connections.

### 1.3 Reclaiming control

Reconnection does not yank control in the middle of a tick. It requests a handoff at the next safe
tick boundary:

1. Revalidate the actor's station assignment and permission.
2. Cancel uncommitted fallback-controller actions and increment the lease epoch.
3. Preserve authoritative unit state, including current heading, speed, targets, heat, and damage.
4. Grant the human controller a new lease.
5. Deliver a structured catch-up report covering elapsed ticks, movement, damage, contacts,
   ammunition/heat changes, actions taken, and the current plan.

The reclaim policy may require an explicit player confirmation when the fallback controller is in
the middle of a hazardous operation such as docking or a cross-domain transfer. It may never keep
control merely because an LLM response is still in flight; that completion becomes stale when the
lease epoch changes.

### 1.4 Anti-exploit invariants

- Connection loss never grants immunity, concealment, free movement, free repairs, or relocation.
- The fallback controller receives the same perception-limited domain projection as the pilot and
  no hidden map truth.
- Its mechanical capability is capped by the unit's automation equipment and configured pilot or
  controller skill; disconnecting must not summon a superior pilot.
- A player cannot change the active fallback policy while offline or after takeover begins.
- Reconnect/disconnect flapping cannot repeatedly reset reaction grace or erase committed actions.
- The linkdead character remains physically aboard and subject to injury, capture, destruction,
  ejection, and other ruleset consequences.
- Every lease change, order, fallback decision, attack, and transition is attributed and journaled.
- Domain replay consumes the recorded controller decisions and lease epochs; it never calls an LLM
  to recreate them.

---

## 2. Allegiance, disposition, and command authority

NPC behavior must not collapse "friendly," "on my team," and "under my control" into one flag.
They are separate relationships with different lifetimes:

```rust
pub struct ForceMembership {
    pub faction: Option<FactionId>,       // persistent political or organizational identity
    pub side: SideId,                     // assignment for this domain or operation
    pub command_chain: Option<CommandId>,
}

pub enum Disposition {
    Allied,
    Friendly,
    Neutral,
    Suspicious,
    Hostile,
    Unknown,
}

pub enum OrderAuthority {
    Binding,       // recipient is expected to obey if the order is legal and possible
    Coordinating,  // recipient should cooperate but retains mission discretion
    Advisory,      // recipient may accept, negotiate, or refuse
    None,
}
```

`FactionId` persists across domains. `SideId` is operation-local and drives coarse IFF and victory
logic. `Disposition` is observer-relative, may be uncertain, and can change as contacts are
identified or rules of engagement are violated. `OrderAuthority` is directional and scoped: being
on the same side does not give every allied pilot control over every unit.

**Grounding note — the faction half already exists.** `mush_faction::FactionId` is a clean, `Copy`,
serde-`transparent`, ordering-friendly public type; "persists across domains" is literally true (it
is monotonic and never recycled). A `ForceMembership` can hold one directly. The one coupling to
weigh: depending on the whole `mush-faction` crate to get `FactionId` also drags in `mush-canon`
and `mush-comsys`; if the sim layer wants only the id, that argues for either a lighter shared id
crate or (cleaner) the sim layer never depending on faction internals and instead **driving faction
state through canon events** — which already works: the faction simulation's clocks advance on
`WorldEvent::CanonRecorded` involving a faction's members (the M25 relevance gate). To make an
engagement outcome move a faction's territory clock, the domain driver emits a canon event of a kind
the clock watches, with entities including the members involved — no new faction plumbing, no
faction↔sim dependency.

This supports four important cases without special unit types:

| Case | Side/disposition | Authority | Expected behavior |
|---|---|---|---|
| Commanded wingman | same side, allied | binding within assigned scope | executes player commander's legal orders |
| Autonomous allied force | same side, allied | coordinating or advisory | pursues its own mission and deconflicts with players |
| OpFor guard or raider | opposing side, hostile under ROE | separate command chain | defends, patrols, raids, or withdraws for its commander |
| Neutral traffic or miner | independent side, initially neutral/unknown | none | pursues an economic mission and escalates only from observed events |

Mission, doctrine, and rules of engagement are also distinct. The mission states what outcome the
controller is pursuing, doctrine supplies preferred methods and risk tolerance, and ROE constrains
what actions are legal. An order may refine the mission but cannot silently bypass equipment,
permission, or ROE checks.

---

## 3. Typed tactical orders

Natural-language radio is presentation, not the mechanical command protocol. A player command or
AI command produces a typed, durable order; the radio line may accompany it for roleplay.

```rust
pub struct TacticalOrder {
    pub id: OrderId,
    pub issuer: ObjId,
    pub recipient: ControllerRef,
    pub issued_tick: u64,
    pub authority: OrderAuthority,
    pub kind: TacticalOrderKind,
    pub target: Option<PerceivedRef>,
    pub constraints: Vec<OrderConstraint>,
    pub expires_tick: Option<u64>,
    pub revision: u64,
    pub status: TacticalOrderStatus,
}
```

Targets use perceived references such as `ContactId`, reported coordinates, formations, or known
landmarks, not unrestricted object IDs. Orders are accepted, rejected, superseded, completed, or
failed with a reason. Binding orders can still fail because they are impossible or illegal;
advisory orders may be refused or negotiated based on mission, doctrine, morale, and trust. Every
status transition is journaled.

---

## 4. Hybrid tactical AI

The tactical controller is a layered planner, not an LLM driving heading and weapons every tick:

```text
domain perception -> structured belief state -> mission/doctrine/orders
       -> deterministic utility planner or bounded LLM deliberator
       -> short-lived TacticalPlan -> deterministic executor -> validated intents
```

The deterministic layer always owns:

- pathfinding, collision avoidance, heading, speed, and formation keeping;
- weapon legality, range, line of sight, heat, ammunition, terrain, and friendly-fire checks;
- emergency reactions, continuation of the current safe plan, and all link-loss fallback;
- conversion of a high-level plan into ordinary domain intents.

Use utility scoring for tactical choice among legal candidate plans and small behavior trees or
state machines for multi-step execution. This is easier to inspect and tune than a single large
decision tree and remains a complete playable controller when no model provider is available.

An LLM is an optional deliberator for ambiguity and characterful judgment: interpreting command
intent, choosing among legal plans, negotiating advisory orders, adjusting risk, responding to an
unexpected contact, surrendering, deceiving, or composing radio traffic. It receives a compact
structured snapshot and must return a schema-validated high-level proposal, for example:

```json
{
  "plan": "flank",
  "target": "contact-7",
  "horizon_ticks": 24,
  "constraints": ["remain_in_sensor_support", "avoid_civilian-zone"],
  "order_response": "accepted",
  "radio": "Two, moving left to screen your advance."
}
```

The proposal is still passed through deterministic authorization and feasibility checks. The model
cannot emit arbitrary MUSH commands, mutate world state directly, or bypass the normal intent
pipeline.

```rust
pub struct TacticalPlan {
    pub controller: ControllerRef,
    pub objective: PlanObjective,
    pub target: Option<PerceivedRef>,
    pub horizon_tick: u64,
    pub constraints: Vec<PlanConstraint>,
    pub source: PlanSource,
    pub revision: u64,
}
```

---

## 5. Bounded LLM latency and stale-result rejection

Variable model latency may contribute human-like reaction time, but it must never determine how
long a domain tick waits. Each deliberation has a deterministic application window:

```text
earliest_tick = request_tick + configured_reaction_delay
deadline_tick = request_tick + maximum_think_window
apply_tick    = first decision boundary at or after completion, clamped to earliest_tick
```

- A fast response waits until `earliest_tick`; buying a faster provider does not create superhuman
  reactions.
- A response received within the window is considered at the next decision boundary.
- At `deadline_tick`, the deterministic planner supplies or continues a plan.
- A late response is discarded, or revalidated as a fresh suggestion only if policy explicitly
  allows it; it never retroactively changes committed ticks.
- While deliberation is in flight, the previous plan and emergency executor continue normally.

Every request carries a decision token containing at least domain, pilot station, controller lease
epoch, request tick, observation revision, order revision, and current-plan revision. Completion is
accepted only if all relevant generations still match and the proposal remains perceived, legal,
and feasible. Human reclaim immediately makes the former controller's outstanding completion
stale.

The accepted plan, its source, and its application tick are committed to the recovery log. Replay
uses that recorded plan and never invokes the model again. Provider timeouts, malformed output,
quota exhaustion, process restart, and disabled LLM service all converge on the same deterministic
fallback path.

**Grounding note — this spine already exists and should be copied, not designed.** The bounded,
schema-validated "flash" request the tactical layer needs is a near-exact copy of the WoD
Storyteller adjudication path (`ruleset-wod/src/st.rs`): `call_structured()` forces the tool when
the provider supports it, falls back to JSON-in-text, does one repair round-trip, and validates by
deserializing into a typed struct; the request schema is bounded (`additionalProperties:false`,
`strict`, integer min/max clamps); and it routes through a dedicated `LlmFeature` (`StAdjudicate`/
`StInterpret`). A `npc_tactical_flash` route is therefore a config block plus one `LlmFeature`
variant and its `route_key` arm — the request/validate/repair machinery is already written (twice —
`st.rs` and `mush-worldgen`). The decision-token / stale-rejection discipline above maps directly
onto the existing correlation-and-revalidate pattern (see §8).

---

## 6. Tactical knowledge and memory

**This is not a new perception system.** The tactical belief state is a *projection of the shared
per-observer epistemic substrate* (the Epistemic Engine: graded, provenance-stamped, per-actor
knowledge of an objective world). "Contact," "confidence," "provenance," and "estimated pose" are
that substrate's vocabulary specialized to tactical sensors; they are the same machinery that
carries Obfuscate, an unlit Umbra, a scried location, or a misidentified face. A tactical contact is
a perceived entity with a provenance chain and a confidence, produced by the same observer-relative
projection that doc 06 §10 uses for rendering. Building a second, tactics-only fog-of-war would
guarantee two systems that disagree; this one must derive from the general substrate even where the
first slice is thin.

No controller, including an LLM controller, receives authoritative map truth. It reasons over the
per-controller belief state:

```rust
pub struct BeliefContact {
    pub contact: ContactId,
    pub classification: ContactClass,
    pub disposition: Disposition,
    pub last_observed_tick: u64,
    pub estimated_pose: EstimatedPose,
    pub confidence: FixedPoint,
    pub provenance: ContactProvenance,
}

pub struct TacticalBeliefState {
    pub contacts: Vec<BeliefContact>,
    pub own_status: PerceivedUnitStatus,
    pub orders: Vec<TacticalOrderView>,
    pub mission_progress: MissionProgress,
    pub known_landmarks: Vec<LandmarkView>,
    pub recent_reports: Vec<ContactReport>,
}
```

Contacts have opaque IDs, confidence, age, and provenance. Reports from allies do not become
perfect knowledge; their age and reliability remain visible. Identification, loss of contact,
sensor error, concealment, and communications limits are ruleset-owned transforms into this
generic envelope — and, per the note above, transforms over the *same* substrate the rest of the
engine perceives through.

Episodic NPC memory remains useful for relationships, promises, fear, reputation, and narrative
continuity. It is not authoritative tactical state. Current position, damage, ammunition, contacts,
orders, and mission progress always come from the structured snapshot, preventing stale prose
memory from overriding the simulation.

---

## 7. Named pilots, automation, and group controllers

A named pilot should normally be an `ObjKind::Npc` physically assigned to the cockpit station. This
allows pilot skill, personality, morale, injuries, ejection, capture, memory, and later human
replacement to work through existing object and station systems. The vehicle remains independent:
destroying, ejecting, disconnecting, or replacing its controller does not change the unit's
identity.

Not every simulated vehicle needs a person object or an LLM. Anonymous traffic, simple guards,
emergency autopilots, drones, and off-map logistics may use deterministic `Automation` controllers.
A formation may have one deliberating leader which publishes typed orders while deterministic
wingmen execute them. This produces coherent lance behavior and bounds cost better than one model
call per unit.

The emergency link-loss controller is always deterministic. If the unit already contains a named
NPC copilot who is permitted and able to take over, the lease may be granted to that actor instead;
their optional LLM planning still follows the same deadline and fallback rules.

---

## 8. Scheduling, activation, and integration with `mush-npc`

Tactical deliberation is stimulus- and salience-driven, never a paid call per tick. Suggested
activation levels are:

| Level | Examples | Controller work |
|---|---|---|
| Abstract | off-map convoy or distant economy | coarse scheduled outcome only |
| Passive | visible neutral traffic, no threat | deterministic route-following |
| Active | nearby contacts, new order, obstacle | utility replanning; optional LLM wake |
| Engaged | combat, surrender, mission crisis | bounded tactical deliberation and executor |

Useful wakes include a materially changed order, contact classification, damage threshold,
blocked route, mission phase, morale state, or plan exhaustion. Minor pose changes do not wake an
LLM.

**Grounding note — reuse the operational pattern, add a new state slot.** The `mush-npc` runtime
already implements the exact discipline this needs and it is richer than the combined draft claimed:
per-NPC coalescing with **one in-flight turn** (`NpcRuntime` with `in_flight`, coalescing window,
and cooldown; `maybe_fire` fires only when `has_mailbox && window_elapsed && cooldown_ok`);
**off-loop model execution** via a snapshot→defer→`tokio::spawn`→re-enter loop (`TurnSnapshot`
serialized into an `Effect::ModuleAsync`, run on a detached task holding no world reference, result
re-entering as `ModuleAsyncDone`); **bounded typed actions** (a fixed `NpcAction` allowlist where an
invented tool name collapses to an inert `UseAbility` that can never become a command); and
**completion-time revalidation** (re-checks the object still exists and is the right kind, re-runs a
move through the same player path, executes abilities on the world loop). Tactical work should reuse
this shape but introduce its **own** `TacticalDecisionSnapshot`/`TacticalPlanOutcome` pair and its
own state slot — the existing coalescing state is keyed per-NPC-`ObjId`, whereas a tactical
controller is often keyed on an engagement, a side, or a formation leader, not a single NPC. Do not
overload `NpcHostState`; the reusable asset is the snapshot→async→revalidate treaty, and it already
has two precedents (NPC turns and WoD adjudication).

If one named NPC participates in dialogue, radio, memory work, and tactical control, its scheduler
arbitrates a shared concurrency and budget envelope. The default priority is:

```text
critical tactical decision > direct radio/order response > ambient dialogue > memory folding
```

Allow at most one paid model request in flight per actor unless later measurements justify more.
Use a short structured-response route such as `npc_tactical_flash` (see §5), bounded input/output
tokens, per-domain and global concurrency limits, and circuit-breaker metrics. Repeated failures
disable only the LLM deliberator; deterministic controller service continues.

---

## 9. Mobility services and lived-in traffic

An NPC-driven hired car is a useful proof that the design is genuinely cross-ruleset — and it is the
recommended **second** proving ground for the whole model (see doc 06's sequencing note), because it
exercises coarse legs, connector crossings, fog-of-war, and promotion-to-live without needing a
combat stack. Examples include a WoD black car or rideshare, a Shadowrun taxi, corporate car,
GridGuide vehicle, courier, ambulance, or smuggling transport. Calling a car creates a transport
mission; it does not summon or teleport an object.

```rust
pub struct MobilityJob {
    pub id: MissionId,
    pub requester: ObjId,
    pub vehicle: ObjId,
    pub driver: ControllerRef,
    pub pickup: KnownPlaceRef,
    pub destination: KnownPlaceRef,
    pub route_policy: RoutePolicy,
    pub phase: MobilityPhase,
    pub requested_at: LogicalTime,
    pub contract: ServiceContract,
}

pub enum MobilityPhase {
    Dispatching,
    EnRouteToPickup,
    WaitingForPassenger,
    EnRouteToDestination,
    Completed,
    Cancelled,
    Failed,
}
```

The customer has scoped service authority to request pickup, destination, stops, waiting, or
cancellation under the contract. They do not receive the movement control lease and cannot command
weapons or demand mechanically impossible routes. The NPC driver may refuse or renegotiate based on
service area, payment, danger, law, faction, trust, mission, or vehicle state.

Dispatch chooses a real available vehicle and driver, then deterministic routing moves it through
the same domains and connectors as everyone else. Pickup and drop-off use ordinary embark/egress
transitions. ETA, traffic, road closures, damage, pursuit, fuel, and detours emerge from the route
and ruleset rather than decorative delay. A model may interpret a vague destination, negotiate,
make conversation, choose among already legal route strategies, or react in character; it does not
run pathfinding.

Full fixed-step simulation is not required for every distant taxi. An off-screen vehicle may use a
coarse scheduled leg, but that leg records its route, time interval, connector crossings, and
exposure points. Surveillance, a player entering the area, a contested checkpoint, or an
interception attempt promotes the relevant leg to an active domain before the encounter boundary.

**The load-bearing invariant, stated as a testable property:** *coarse travel may reduce fidelity,
but it must never skip a registered observation or interception opportunity, and must never produce
instantaneous cross-city relocation.* This is not prose — it is a property to test the way doc 06
tests the placement-graph invariants. Model each connector crossing and exposure point on a leg as a
durable scheduled checkpoint on the due-time scheduler; promotion-to-live is then deterministic and
provably cannot miss a checkpoint, because reaching one is what triggers the promotion.

Third parties learn about the vehicle only through ordinary in-world channels: seeing a contact,
watching a pickup, compromising dispatch communications, tracking a beacon, bribing a dispatcher,
or covering a connector. The service API must not reveal destination, route, passenger identity, or
true pose to an unauthorized observer. This turns transport time into useful gameplay for tails,
ambushes, escapes, protection work, and counter-surveillance while preserving fog of war. Route
knowledge is itself perceived, provenance-stamped knowledge in the epistemic substrate (§6): who
learned a route, how, and how stale it is are first-class, not free.

---

## 10. Testing strategy (tactical)

These extend doc 06 §14. They assume its foundation scenarios (1–13) already pass.

1. Lose the pilot's last connection; verify the unit remains on-map, keeps safe motion, and the
   configured fallback controller takes over only after its control grace.
2. Reconnect before takeover and after takeover; verify cancellation or lease-boundary reclaim,
   stale-intent rejection, and the structured catch-up report.
3. Repeatedly flap a connection; verify the grace cannot be extended indefinitely and every
   control epoch replays deterministically.
4. Exercise continue-to-destination, return-to-origin, maximum-speed retreat, fighting retreat,
   self-defense, and stand-and-fight policies without granting hidden perception or relocation.
5. Place a commanded wingman, an autonomous ally, an OpFor guard, and a neutral hauler in one
   domain; verify that side, disposition, and order authority remain independent.
6. Issue binding, coordinating, and advisory orders; verify typed acknowledgement, refusal,
   supersession, expiry, and replay without parsing presentation radio text.
7. Return equivalent LLM plans before the reaction delay, inside the decision window, after the
   deadline, and after a lease/order revision; verify deterministic apply ticks, fallback, and
   stale-result rejection.
8. Give an NPC incomplete and erroneous contact reports; verify its deterministic and LLM plans
   use only the belief state and never reveal an underlying object id or true hidden pose.
9. Run a lance with one deliberating leader and deterministic wingmen; verify typed group orders,
   bounded model calls, and continued service during provider failure.
10. Put one named pilot under simultaneous tactical, radio, ambient-dialogue, and memory stimuli;
    verify priority, coalescing, budget enforcement, and at most one paid request in flight.
11. Request a WoD black car from elsewhere in the city; verify dispatch selects an available
    vehicle, the car travels to pickup, the passenger embarks, and the car follows a real route to
    the destination without teleportation or passenger movement authority.
12. Place a lawful surveillance or interception opportunity along that route; verify off-screen
    travel promotes to active simulation in time, authorized observers receive only perceived
    contacts, and a provider outage does not prevent deterministic arrival or interception.

---

## 11. Implementation phases (tactical)

These follow doc 06's Phases A–E. Note the sequencing recommendation in doc 06 §16: the mobility
slice (§9) is pulled **forward** as the second cross-ruleset proving ground, ahead of the full
TacSim tactical combat, because it validates the generic layer (coarse legs, connectors,
fog-of-war, promotion) without a combat stack.

### Phase F: tactical interaction and scenes

- Add terrain costs, spatial indexes, LOS/contact projections, sensors, weapons, heat, and damage in
  ruleset-owned increments — building tactical contacts as a projection of the epistemic substrate
  (§6), not a parallel system.
- Add deterministic utility planning, typed missions/orders/ROE, controller belief states, and
  formation-level command.
- Bind real-time engagement scenes to domains and journal semantic commits (doc 06 §11).
- Add deck-plan domains only after the one-domain model is stable.

**Exit criterion:** a recorded engagement can be replayed mechanically and narratively without
leaking hidden map state.

### Phase G: NPC forces and bounded LLM deliberation

- Add persistent faction membership (reusing `mush_faction::FactionId`), domain-local sides,
  observer-relative disposition, and scoped command authority.
- Add named NPC pilots and deterministic automation controllers using the same control leases.
- Add salience activation, shared NPC job arbitration (reusing the `mush-npc` treaty; new state
  slot), structured tactical snapshots/outcomes, and bounded decision windows.
- Add one generic mobility-service scenario with dispatch, coarse off-screen route legs, active
  promotion, pickup/drop-off, and interception hooks.
- Add an optional flash-model route (copy `st.rs`) only after the deterministic controller passes
  the same scenarios without a provider.

**Exit criterion:** tactical scenarios 1 through 12 pass with the model route enabled, disabled,
slow, malformed, and unavailable; the resulting committed plan trace remains replayable.

---

## 12. Decisions and rationale (tactical)

These were D8 and D9 in the combined draft.

### D1. Link-loss timing defaults

| Policy | Possible default | Tradeoff |
|---|---|---|
| Immediate full takeover | safest mechanically | treats a momentary network fault as a deliberate handoff |
| Reuse the 300-second session grace | simple | leaves tactical control unattended far too long |
| Immediate safety plus a short separate takeover grace | protects the unit while preserving quick reconnect | adds one interruption state machine |

**Decision:** use these production defaults:

```text
last connection lost unexpectedly: deterministic safety immediately; full takeover after 15 s
clean QUIT or administrative boot: deterministic full takeover at the next domain tick
stable reconnection before a later loss earns fresh grace: 60 s
human reclaim: next safe tick boundary
```

The 15-second value is a staff-configurable domain class setting, clamped to `0..=30` seconds; it
is not player-configurable. The deadline is converted to the first eligible domain tick and its
availability generation is journaled. During grace the safety executor continues the last legal
plan and emergency reactions, so the vehicle is never uncontrolled. This sim grace is hung off the
new **linkdead-onset** event (§1.2), which fires while the character is still `is_connected` — it is
deliberately shorter than and unrelated to the existing 300-second session grace, which governs
whether the login session itself survives.

### D2. Ownership of link-loss policy choices

| Option | Advantages | Problems |
|---|---|---|
| Players choose every objective and engagement mode | expressive | permits unsupported or exploitable automation behavior |
| Staff/doctrine fixes everything | predictable | removes meaningful unit preparation and commander preference |
| Layered allowed set with a sortie-time player choice | expressive within enforceable limits | requires policy resolution and clear UI |

**Decision:** resolve policy in this order, where each layer may only narrow the previous one:

1. Staff/world policy defines allowed objectives, engagement modes, grace bounds, prohibited
   areas, and hard safety rules.
2. Installed equipment and the assigned fallback controller define actual capabilities.
3. Faction doctrine, mission, and ROE narrow the allowed set and provide defaults.
4. While connected, the owning player or authorized commander chooses from the remaining set.
5. Launch snapshots the resolved policy into the sortie record. It cannot be changed after the
   interruption episode starts.

Players may normally select `ContinueCurrentPlan`, `ContinueToDestination`,
`ReturnToSortieOrigin`, `SeekNearestFriendlyRecovery`, or `HoldPosition` when routing and equipment
support them. They may select `EvadeAtMaximumSpeed`, `FightingRetreat`, or `DefendSelf`.
`StandAndFight` requires combat-capable automation plus explicit world/doctrine permission and means
defend the current mission or locality against contacts legal under existing ROE; it does not allow
the fallback controller to invent targets or begin a new offensive mission.

Recommended templates are:

| Sortie type | Objective | Engagement | Terminal fallback |
|---|---|---|---|
| Player combat unit | return to sortie origin | fighting retreat | hold and defend self |
| Civilian/logistics unit | continue to destination | evade at maximum safe speed | seek friendly recovery |
| Unit without navigation automation | hold position | defend self | remain under emergency safety control |

Autonomous allied, neutral, and OpFor units use their own mission/doctrine policy rather than a
player link-loss template. All controllers remain subject to perception, skill, equipment, fuel,
heat, damage, connector access, and ordinary travel time.

Exact tuning constants (grace seconds, activation thresholds, token budgets) are chosen by load
tests and telemetry, without reopening the ownership and safety boundaries above.
