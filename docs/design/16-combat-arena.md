# 16 — Combat: the arena, the story, and what stacks on it

Draft 0.2, 2026-09-16, written from the answered survey [16a](16a-combat-owner-questions.md) against
`main` at `5da8d6a` and revised after the three-lens review recorded in [16b §10](16b-combat-implementation-plan.md).
The decisions this document rests on are collected in §12 as D-16-1 to D-16-20 (owner, 2026-09-16,
binding; D-16-19 and D-16-20 added by the review).

Reading order for someone new: §1 (what "done" means), §3.1 (the arena, its seam and its tempo), §3.4
(motion and the pulse), §3.5 (how a shot resolves), §7 (where the code goes), §12 (the decisions).

---

## 1. Purpose and the two acceptances

This document specifies the first combat build of the single 2350-era TacSim game: a simulation arena
off the OOC Area in which two players in tanks find each other and fight until one is destroyed. It is
written as the basis for everything that follows (lasers, bipeds, more sensors, ECM, ECCM, elevation,
then real-world fights), so each of those is an addition specified in §9, never a rewrite.

Two acceptances define "done", both on the production composition:

**A1, the story test.** Two connected accounts create characters, walk from the OOC welcome hall into
the simulator lobby, pick opposite teams, board their pods, receive the default tank, start it up, drive
across terrain, see each other only once line of sight and sensor range allow, lock, fire cannons, and
one tank is destroyed. The match records a result, both pilots leave, and a second match can start. One
test in `mushd/tests` (in-process) and its twin in `mush-server` through the gateway boundary; both must
pass, because the output path differs (§3.1).

The observer rule is asserted surface by surface, never as one blanket claim: before detection the
other tank appears in none of the cockpit `VIEW` frame, the atlas `+map` frame, `look` in any room,
`@unit/status` and `@unit/show` for the other pilot, contact notices, the TUI panels, and the web feed.
The money rule: nothing in the campaign economy changed.

**A2, the scale harness.** 300 units placed and moving on the arena, 24 of them fighting, 150 weapons on
their cadences, one sensor pass per unit per second of arena time, at least 12 connected fake sessions
receiving output, the LLM stubbed, on one core: per-pulse p99 under 25 ms in `Engaged` tempo, resident
memory flat over one hour of arena time with the ledger streamed out (§3.1), the streamed ledger under
`harness.max_ledger_bytes`, traces and cell probes per pulse under their bounds, path queries per pulse
under their bound. The harness runs at the configuration maxima of every world fact in §2. Where it runs
and whether it gates is D-16-19.

---

## 2. World facts the engine relies on

Ratified 2026-09-16 (16a §9). Each fact bounds an engine cost, has a configuration knob, and is asserted
by the harness. They belong in the setting docs beside the Retreat and Concordat material; here they are
stated as engine requirements.

| # | Fact | Engine requirement | Knob | Harness assertion |
|---|---|---|---|---|
| W1 | Standard ECM defeats detection at long range | Sensor ranges are short and bounded | `sensors.<class>.max_range_cells`, `sensors.max_signature_factor` | Candidates per pass bounded by units within `max_range × max_signature_factor` |
| W2 | Lasers kill drones and soft fliers, not armour | No drone swarms; projectiles primary | `arena.max_units_by_class` | Class counts on the map obey the caps |
| W3 | Active sensors are suicide; doctrine is passive | A pass is range plus LOS, nothing else | `sensors.<class>.active = false` | No emitter term in detection |
| W4 | Lasers burn optics in the open; fights are short-range in clutter | Traces are bounded per pulse; arenas are authored with clutter | arena content | Traces and cell probes per pulse under `harness.max_traces_per_pulse` |
| W5 | Transponders are spoofable; identification takes a close look | Classification is a slower cadence than detection | `sensors.classify_after_ms` | Classifications per pulse bounded |
| W6 | Tactical comms are LOS mesh jammed by the same ECM | Shared contacts fan out only within comms range | `comms.share_range_cells` | Share operations per pulse bounded |
| W7 | Licensed minds are leased over the relay | Deliberation is optional; reflexes always run | `[llm.routes.tactical_flash]` | Harness runs with the LLM disabled |
| W8 | The Compact caps span of control and needs a human trigger | Bounded AI per commander; fire is a discrete act | `command.max_span` | Orders per commander per minute bounded |
| W9 | Heavy hulls are scarce post-Retreat | Twelve a side is the setting's ceiling for a fight | `match.max_slots_per_side` | Fighters at 24 |
| W10 | The sky is a laser kill zone | Ground only on the tactical map; elevation is terrain | driver `media = ground` | No `air` medium participant |
| W11 | Contested ground comes in Interval pockets joined by corridors | One arena per fight; corridors are journeys | one arena per domain | One region per fight |
| W12 | Armour is modular plate by facet | Facet attribution; no impact geometry | facet table in `UnitSpec` | No geometry code in the impact path |
| W13 | Indirect fire needs a spotter net ECM breaks | Direct fire only | none | No `indirect_fire_path` consumer |
| W14 | Interval-grade power plants | One to fifteen second cycle times | `weapons.<id>.cycle_ms` floor 1,000 | Shots per pulse bounded |
| W15 | Ammunition is logistics | Short engagements; units withdraw | `units.<id>.ammo`, `match.max_duration_ms` | Match length bounded in the harness |
| W16 | A battle's canon is reconstructed from black boxes | The match ledger is an in-world artifact | ledger streamed and digested | Ledger complete and digest-matched for a harness match |

Doc 10 §8's combined-arms list is read as strategic and orbital aerospace untouched; tactical-map air is
what W2 and W10 remove; "drones scout" becomes "drones scout until a laser sees them", which keeps them
as later content.

---

## 3. The model

### 3.1 Time, tempo and authority

**The arena is an authority.** One arena per map. While a match runs, the arena is the single writer
for its own facts: unit poses, unit condition cells, contacts, orders, scheduled impacts, the match
ledger. The world task remains the single writer for everything else: identity, focus, leases, pods,
rooms, the economy, and the durable records. The two share no state and no lock (D-16-1). Doc 06 §6.3 is
amended accordingly (§11).

**The seam is seven messages and nothing else.**

```text
world -> arena   CustodyOut { definition, units[{cells, pose}], pilots[{unit, station, epoch}], seed }
world -> arena   Intent     { unit, channel, epoch, kind, seq, payload }        (bounded inbox)
world -> arena   Control    { unit, channel, epoch, state: Live | Linkdead | Released }
world -> arena   Stop       { reason: Abort | Timeout | Void }
arena -> world   Ledger     { chunk }                                            (bounded egress)
arena -> world   Frames     { pilot -> frame }                                   (latest-wins per pilot)
arena -> world   CustodyIn  { units[{cells, final_pose, outcome}], result, ledger_digest, reason }
```

- `Intent` is validated at the world (focus, lease, channel, epoch) when enqueued and **authorised at
  apply** in the arena against the arena's own per-`(unit, channel)` epoch table (doc 09 C8). The world
  raises that table through `Control` on every seat activation, deactivation, lease revocation,
  linkdead onset and revival (doc 07 §1.2), `EXIT` and `SIM ABORT`; the arena purges every queued intent
  below the new epoch. A pilot who types `FIRE` then `EXIT` in one burst never fires.
- `Ledger` chunks (16 KiB) leave the arena as they fill; the world appends them to a
  `mush_core::Ledger` (`log:match.<id>`) on its own writer lane, ack-gated, best effort during the
  match. The arena's egress is a bounded ring; if the world cannot keep up the arena marks itself
  `Lagged`, and past `ledger.max_unflushed_chunks` the match is stopped with `Void`. The arena never
  holds a store handle (D-16-3).
- `Frames` are one latest-wins slot per pilot. The world task drains them each iteration and emits them
  through its ordinary output path addressed as `SendTarget::Char`, with the loop's own policing and the
  gateway resolving connections; the arena never touches `GatewaySink`. A slow world drops stale
  frames, never events.
- `CustodyIn` is applied per unit and per-unit fallible (§Custody).

**The arena core is a library.** `tacsim::arena::Arena` holds the state and exposes
`step(now_ms, controls, intents) -> StepOutput { ledger_chunks, frames, tempo, next_due_ms }`, a pure
function of its state and inputs with no I/O, no task and no clock of its own. The arena task is a thin
wrapper that owns an `Arena`, the inbox, the egress ring, a timer driven by `next_due_ms`, a watchdog and
`catch_unwind`. Tests and the harness call `step` directly.

**Tempo** is adaptive per arena (D-16-2):

| Tempo | Period | Enter when | Leave when |
|---|---|---|---|
| `Quiet` | 5,000 ms | default; no hostile contact held by any unit, no pending impact, no fire intent, no AI in `Approach` | a threshold below is crossed |
| `Alert` | 1,000 ms | any unit holds a contact (classified or not) whose Euclidean range is under `tempo.alert_range_cm`, or a fire intent is queued, or an AI enters `Approach` | `tempo.alert_hold_ms` (30,000) with none of those |
| `Engaged` | 500 ms | a shot is fired, an impact is scheduled, or hostile units are within weapons range of each other | `tempo.engaged_hold_ms` (10,000) since the last shot, impact or in-range pair |

Tempo changes are ledger events. The harness's 25 ms p99 applies in `Engaged`.

**Rate independence** (D-16-2). The tempo decides only how often `step` runs and how output is batched;
it never decides an outcome. That is achieved by one rule: **a pulse is a sequence of substeps, and
every due event is applied at its own arena timestamp between substeps.**

- The substep length is `min(tempo.substep_max_ms, cell_cm × 1000 / max_speed_cms_of_any_unit)`,
  floored at 50 ms, so no unit ever advances more than one cell in a substep (§3.4).
- Scheduled impacts, sensor passes, classification transitions, startup completions and weapon-ready
  times are events with arena timestamps; each is applied in timestamp order at the first substep
  boundary at or after its time, against the positions at that boundary.
- Every random draw is derived from `(match_seed, label)` with an enumerated label (§3.5); there is no
  stream whose position depends on how many pulses have run.
- All cadences, cycle times, delays and holds are in arena milliseconds.

The property that pins this: one scripted intent and control log replayed at 5,000, 1,000 and 500 ms
produces an identical ordered event list and identical per-substep state hashes (16b A7).

**Clock and lag.** Arena time is milliseconds since match start, advanced by wall time between pulses
clamped to `max_catch_up_steps × period` (doc 06 §6.4, initially 4). Beyond that the arena records
`lagged` and skips idle wall time; it never expands `dt` to hide lag.

**Pulse phases, in order, each reading the state the previous phase committed** (doc 06 §6.3):

1. Apply `Control` messages to the epoch table; apply `Stop` if present.
2. Drain `Intent`s: authorise each at apply; continuous controls set desired values (latest wins);
   discrete actions are queued per unit, bounded at `intents.max_pending_per_unit`.
3. For each substep in order: (a) integrate motion for every unit from the substep-start snapshot, in
   unit-id order; (b) rebuild the spatial index; (c) apply every due event with timestamp at or before
   the substep end, in timestamp order then event-sequence order: impacts (damage), weapon-ready,
   startup-complete, sensor passes (detection and classification), share operations.
4. Resolve newly queued discrete actions that became legal this pulse (`LOCK`, `FIRE`, `STARTUP`),
   scheduling their events.
5. Evaluate tempo; compute the state hash; append the pulse's ledger events; build frames for pilots
   whose view changed; compute `next_due_ms` (the earlier of the next period boundary and the next due
   event).

**Determinism** (D-16-4). Authoritative arena state is integer: centimetres, millidegrees,
milliseconds, cm/s, per-mille probabilities. Floating point is allowed inside a computation as long as
every stored result is quantised. Each substep produces a state hash recorded with its pulse.

**Durability** (D-16-3). For OOC matches the arena holds all state in memory and never touches the
store. The ledger is streamed (above) so memory stays flat and the black box survives, but a crash still
voids the match: there is no resume. The result record (§4) holds the ledger's digest and log id, not
the ledger. Per-pulse state durability and resume are requirements before IC play (§11).

**Custody** (D-16-1, D-16-20).

- *Check-out* happens at `Boarding → Running` (§3.8). The world task, in one authoritative action: writes
  on each unit record a `deployed { arena, snapshot }` marker where `snapshot` is the unit's cells and
  pose at check-out; leaves the unit's world placement in place, frozen (so the pilot's channel grants,
  which require a placed unit, survive); refuses ordinary mutation of a deployed unit at the real
  admission sites (`TacticsDestroyGate::check`, `TacticsDetachGate::check`, `unit_damage`,
  `unit_remove`, `@spatial` moves) with one named error; excludes the arena's domain from every world
  map and unit listing (§3.6); and sends `CustodyOut`.
- *Check-in* happens once, at `Ended`, `Aborted` or `Void`. Per unit, fallibly: the returned cells are
  applied as one `UnitMutationPlan` through `apply`, the final pose through `mush_sim::place`, both in
  the same authoritative action; a unit that no longer exists is journalled `custody_lost` and skipped,
  never failing the others. Then the marker is cleared, the tank is reset (§3.3), and seats, leases,
  focus and pods are released. On `Void` (panic or ledger overflow) the world restores each unit from
  the check-out snapshot and commits no arena-derived cell.
- *Boot reconciliation.* The tactics module's recovery pass treats any `deployed` marker with no live
  arena as a voided match: restore the snapshot, clear the marker, release seats, leases, focus and pods,
  and write an `aborted(restart)` result. Hot restart of `mush-server` under the gateway is routine, so
  this path is exercised by a test that restarts the server mid-`Running`.

**Failure.** `step` runs under `catch_unwind`; a panic voids the match (`CustodyIn` with `Void` and no
cells) and the world continues, consistent with the workspace's panic firewall (nothing a faulted turn
touched reaches the store). A pulse exceeding `tempo.stall_ms` (2,000) is voided the same way by the
watchdog.

### 3.2 Space

**Pose is integer** (D-16-4): `Pose { x_cm: i64, y_cm: i64, z_cm: i32, heading_mdeg: u32, posture }`
with `heading_mdeg` in `0..360_000` clockwise from north. The `@spatial/place` wire keys become
`x_cm, y_cm, z_cm, heading_mdeg`; the legacy pose wire arm is deleted. Authored packs keep metres in JSON
and convert at load. `z_cm` is carried from the first build and equals the cell's `elev_cm`, zero on the
first arena.

**Range** is one function used everywhere: Euclidean centimetres between two poses. Range bands,
angular size, flight time and the tempo trigger use it; the atlas `Trace` supplies cover and blocking
only, never distance (its `range_cells` is Chebyshev).

**The arena window** is one `mush-sim` domain over one region surface at `cell_cm = 1000`, 256 by 256
cells (D-16-5): 2.56 km square, the current `MAX_TILE_SIDE`. The path to 10 km is a per-driver tile side
and a tiled composite, specified in §11 as an addition and re-run through the harness first.

**Terrain** is the existing 18-class cell grid with the engine's cover and opacity per class. The first
arena is authored with roads, open ground, light and heavy woods, shallow water, a few buildings, two
team entry zones, three or four named key spots, and a waypoint graph for the reflex controller (§6).

**Line of sight** is the atlas `line_of_sight` trace over the shared window with `Eye{cell, height_cm}`
from the unit definition's `sensor_height_cm` (new field; 150 by default). It is directional; there is
no cross-observer cache. Each observer's pass traces its own candidates (§3.6), and the harness bounds
traces and cell probes per pulse. **Direct fire** is the atlas `direct_fire_path`, whose `target_cover`
finally has a consumer (§3.5).

**The spatial index** is a per-arena uniform grid of cell buckets over unit positions, rebuilt every
substep, used for range culls, in-range pair detection, impact candidate lookup and occupancy.

**Edges** stop motion. A unit cannot leave the window.

### 3.3 Units

**Unit truth is M32** (D-16-6): `TacticalUnit` with sections, facets, `ScalarCell`s, components,
capabilities and a `MobilityProfile`; all damage is a `UnitMutationPlan` through `apply`; refuse, never
clamp. No combat-specific unit type exists.

**The stock tanks are persistent objects.** Eight tank objects, one per pod, live in the arena's domain
and are created once by the centre's content (§4). At check-in and at every `Idle` transition each is
reset to its `UnitSpec` by one `UnitMutationPlan` (cells to initial values, conditions to `intact`,
capabilities restored). No objects are created or destroyed per match.

**The stock tank** (`line_tank`, one definition):

| Section | Facets (armor cells) | Section cells | Consequence at zero internals |
|---|---|---|---|
| `hull` | `front`, `left`, `right`, `rear`, `top` | `internals` | `destroy_unit` |
| `turret` | `front`, `left`, `right`, `rear` | `internals` | `remove_weapons(turret)` |
| `left_track` | `left` | `internals` | `slow(50)`, `no_turn_toward(left)` |
| `right_track` | `right` | `internals` | `slow(50)`, `no_turn_toward(right)` |
| `engine` | (none; struck through `hull.rear`) | `internals` | `disable_motion` |

Components: `cannon` (turret; cells `ammo`, `ready_at_ms`), `machine_gun` (turret, coaxial; cells
`ammo`, `ready_at_ms`), `visual_sensor` (turret; cell `condition`), `power` (hull; capability
`tacsim.power` with state `off | starting | online`). `sensor_height_cm` and the silhouette table are
unit-level fields. Every number lives in the definition pack (§4).

**A wreck** is a defined M32 state: `hull` condition `wreck` (a named `ConditionId`) plus
`RevokeCapability` of `tacsim.power`, `tactics.mobility` and every `tacsim.weapon.*` capability, applied
by the `destroy_unit` consequence in one plan. The game-crate predicate `is_wreck(unit)` reads exactly
that. A wreck is an obstacle and a contact, never a target.

**Facet attribution** (W12): the impact's relative bearing selects a facet band; a table in the
`UnitSpec` maps each band to the sections it can strike with weights. The default sends `front` to hull
and turret, `rear` to hull, engine and turret, sides to hull, the near track and turret, `top` only to
hull and turret. No geometry is computed.

**Silhouette**: each facet band declares a presented width and height in centimetres.

**Mobility mode rules** for `tracked`: `max_speed_cms`, `accel_cms2`, `brake_cms2`, `turn_rate_mdeg_s`,
`reverse_factor_pct`, decoded from the opaque `ModeRules` by the game crate.

**Startup**: `STARTUP` sets `power` to `starting` and schedules `online` at `now + match.startup_ms`
(10,000) as a due event; until `online`, motion and fire intents refuse with a named error; repeating
`STARTUP` is idempotent.

**Stations are on the tank.** The pod's station record lives on the tank object (`stations` and
`assignments` are keyed by unit), so the `helm`, `sensors` and `fire` grants have the tank as subject.
The pod is where the body sits.

### 3.4 Motion

Per unit per **substep** (§3.1), with `dt` the substep's milliseconds:

1. `heading` turns toward the desired heading at `turn_rate_mdeg_s`, shortest arc, blocked toward a
   destroyed track's side.
2. `speed` moves toward the desired speed at `accel_cms2` (or `brake_cms2` when reducing), capped by
   `max_speed_cms × terrain_factor × slow_consequences`, where `terrain_factor = 100 / move_cost` of the
   cell the unit is in, for `tracked`.
3. The position advances `speed × dt` along the heading in centimetres. By construction of the substep
   this crosses at most one cell boundary.
4. If a cell boundary is crossed, the cell entered is tested: impassable for `tracked`, occupied by
   another unit, or off the window stops the unit at the boundary with `speed = 0` and one `blocked`
   notice per stop. **Invariant: no unit ever occupies a cell that was not tested on entry.**
5. Occupancy is one unit per cell at 10 m cells; there is no ramming and no collision damage.

Disabled units stay on the map as obstacles and contacts. Wrecks (§3.3) likewise.

### 3.5 Weapons, resolution, damage

**Weapon definition** (§4): `delivery: direct | beam`, `mount` (section and facet), range bands with
`max_range_cm` and a per-band `dispersion_mrad`, `cycle_ms` (floor 1,000, W14), `ammunition`,
`burst_rounds` (1 for the cannon, `k` for the machine gun), `muzzle_speed_cms`, `heat_per_shot` (0 for
both stock weapons; the cell exists for lasers).

**Ammunition definition**: `damage` (one fixed value), `penetration_class` (carried, unused by the
stock rules), `count_per_load`.

**Draw labels** (doc 09 C5). Every draw is `draw(match_seed, label)`, a keyed hash, with these labels
and no others: `hit:<shot_seq>`, `facet:<shot_seq>`, `burst:<shot_seq>`,
`classify:<observer>:<target>:<pass_seq>`. Adding a draw can never shift another. The label set is
pinned by a contract test, and the arena is recorded as exempt from `provider_rng_policy` because it is
not a kernel action provider.

**Resolution** (D-16-8, 16a Q33), at the moment `FIRE` is accepted:

1. Presented silhouette: the target's facet band toward the shooter gives width and height; cover from
   `direct_fire_path.target_cover` scales the presented area (`Partial` halves it, `Full` zeroes it).
   Angular size in milliradians is `width_cm × 1000 / range_cm`.
2. Error cone: the weapon's `dispersion_mrad` for the band, plus the terms of the named config table
   `resolution.error_terms`: shooter speed, target lateral speed, gunnery (every stock rating is 1
   through the `PilotProfile` seam), sensor condition.
3. `p_permille = hit_curve(angular_size_mrad × 1000 / error_mrad)`, a named monotone table in config
   with interpolation, clamped to `[resolution.p_min, resolution.p_max]`; zero presented area is zero.
4. `u = draw(seed, hit:<shot_seq>) mod 1000`; hit if `u < p_permille`. A burst of `k` rounds is one
   binomial sample from `(k, p)` using `burst:<shot_seq>`.
5. The cockpit shows `p_permille` as a percentage in `WEAPONSTATUS` and on `LOCK` before any shot.

**Acceptance and cost.** `FIRE` requires power online, a lock on a contact the platform holds, a ready
weapon and ammunition. A shot whose line is blocked at acceptance, or whose target is behind full cover,
is **accepted**: it consumes ammunition and the cycle exactly like any shot and resolves as a `miss`
with the blocker as its reason. There is no free probe of line of sight.

**Scheduled impact.** An accepted shot sets `ready_at_ms`, decrements `ammo`, records `fired`, and
schedules `impact_at_ms = now + range_cm × 1000 / muzzle_speed_cms`. At impact (a due event, §3.1) the
arena re-validates: the target exists and is not a wreck, its position is within one cell of the
prediction (a target that changed facet band re-resolves with the same `hit` draw), the line is clear.
A failed validation is a `miss` with its reason. Beams (later) have `impact_at_ms = now`.

**Damage.** A hit selects a section from the facet table by weight (`facet:<shot_seq>`), reduces the
facet's `armor` cell by `damage` down to its floor, applies overflow to the section's `internals`, and
at zero internals sets the section `destroyed` and applies its consequence. All of it is one
`UnitMutationPlan` per impact through `apply`, journalled, with one `hit` ledger event carrying the
plan's cause. `destroy_unit` makes a wreck (§3.3) and, when it eliminates a side, ends the match (§3.8).
Doc 10 3A holds: disabled precedes destroyed; capture and salvage are not arena concepts.

### 3.6 Perception

**Reused as vocabulary and projection rules, not as storage** (D-16-10): the contact lifecycle
(`observe | refresh | share | unshare | loss | reacquire`), the refresh and TTL bands, and the
existence-hiding rule that an unauthorised viewer's answer is byte-identical to a nonexistent unit's.

**Arena contacts are an in-RAM shadow with their own identity space.** The arena mints one contact id
per `(viewer platform, target)` as a random 64-bit value from `draw(seed, contact:<viewer>:<target>)`,
never sequential, never reused within a match, formatted as the existing opaque hex. It is a distinct
type from `mush_core::ContactId` and never crosses to the durable `tactics/contacts` family, which the
arena cannot write. Any id a pilot names that is unresolvable, unowned or expired gets one
byte-identical refusal. Contacts die with the match; the ledger's `detected`, `classified` and `lost`
events are the record.

**The pass.** Each unit's pass is due every `sensors.pass_ms` (1,000) of arena time, staggered by
unit id, and runs as a due event at its timestamp against the positions at that substep boundary:

1. Candidates: units within `sensors.visual.max_range_cells × sensors.max_signature_factor` of the
   observer through the spatial index, excluding wrecks unless `sensors.visual.sees_wrecks`.
2. For each candidate, `line_of_sight` from the observer's `Eye` to the candidate's `Eye` must be
   `Clear` or `Partial` with attenuation under `sensors.visual.max_attenuation`.
3. Range test: `range_cm ≤ max_range_cm × signature_factor(target class, posture)`; a stopped tank in
   heavy woods has a lower factor than one moving on a road.
4. A candidate that passes becomes or refreshes a contact with `class = unknown`; after
   `sensors.classify_after_ms` of detection with no gap longer than `sensors.pass_ms` it becomes
   `classified` with the unit's class and team (`classify:<observer>:<target>:<pass_seq>` if the rules
   ever need a draw). Loss follows the TTL.
5. Team sharing (W6): a unit's classified contacts are shared with teammates within
   `comms.share_range_cells` and LOS, as `share` operations bounded per pulse.

**World-side exclusion.** While an arena holds custody, its domain is excluded from `visible_rooms`,
`units_for`, the atlas `+map` frame, `look`, and every world unit listing, for every viewer including
staff. A deployed unit's `unit.status` answers "deployed" to a viewer stationed on it and the
existence-hiding line to anyone else. The only views of arena state are the cockpit frames the arena
builds per pilot.

**What a pilot sees**: `CONTACTS` lists the platform's contacts with opaque id, class or `unknown`,
bearing, range band, age; notices fire once per transition (`acquired`, `classified`, `lost`) with
hysteresis; `VIEW` renders the arena window from terrain plus the platform's own contacts. Teammates
appear when shared. Nothing else about another unit crosses to a pilot before detection, on any
surface, and A1 asserts it surface by surface.

The ECM and ECCM hooks (§9) are two null fields on the sensor and unit rows today: `emission` and
`jamming`.

### 3.7 Control and the cockpit

**Pods** are Things that are interior spaces (`SpatialParent::Interior{space: pod, slot}`), eight per
centre, four per team room; the lobby, red room and blue room are three rooms off the OOC welcome hall,
authored in the atlas-owned OOC neighborhood pack at its next version (§4).

**Boarding** (`BOARD`, contextual in a team room for a pilot who holds a slot) places the character in
the pod, assigns the tank's station to the character, activates the seat with channels `helm`,
`sensors`, `fire` through `request_seat_activation`, refreshes actor links, and moves the account's
focus to the station. `EXIT` reverses it and sends `Control{Released}`.

**Verbs** (D-16-11), contextual while focused on a station, each a selector that stages one intent:

| Verb | Arguments | Refusals |
|---|---|---|
| `STARTUP` | none | none (idempotent) |
| `HEADING <deg>` | 0 to 359 | power not online |
| `SPEED <kph>` | 0 to max | power not online |
| `VIEW` | none | none |
| `CONTACTS` | none | none |
| `LOCK <contact>` | opaque id | one byte-identical refusal for any id the platform does not hold |
| `FIRE [<weapon>]` | default cannon | no lock, cycling, empty, power not online |
| `WEAPONSTATUS` | none | none |
| `STATUS` | none | none |
| `EXIT` | none | none |

Lobby and team-room verbs: `SIM TEAM red|blue`, `SIM LEAVE`, `SIM START`, `SIM ABORT`, `SIM STATUS`,
`BOARD`. The shipped `mush-sim` combat verbs (`+fire +launch +heading +thrust +posture +dc +dock
+eject`), their `tactical_intent` staging and the placeholder resolution are retired with this build
(D-16-20); ships and other domains later get verbs on the same arena model.

### 3.8 Lifecycle

```text
Idle → Selection → Boarding → Running → Ended → Idle
                 ↘        ↘        ↘
                  Aborted ← Aborted ← Aborted | Void → Idle
```

- `Selection`: `SIM TEAM` claims a slot; one slot per account; two to `match.max_slots_per_side` per
  side; no budgets, terms or consent (16a Q28).
- `Boarding`: `SIM START` (a team lead or staff) with at least one pilot per side; pods lock to their
  pilots; pilots `BOARD`. The transition `Boarding → Running` fires when every claimed slot is seated or
  `match.board_timeout_ms` elapses with unseated slots dropped (an empty side aborts). Check-out happens
  on that transition.
- `Running`: the arena steps. Team elimination (every unit of a side a wreck or disabled with no weapons)
  ends the match; `match.max_duration_ms` ends it as a timeout; `SIM ABORT` by any participant or staff
  aborts it; a panic or ledger overflow voids it.
- `Ended | Aborted | Void`: check-in (§3.1); result written; tanks reset; seats, leases, focus and pods
  released atomically; the reset is idempotent and re-runnable by staff. An abort from `Selection` or
  `Boarding` releases slots and pods only; there is no custody to return.
- Disconnects: on linkdead onset the world sends `Control{Linkdead}`; the tank keeps its continuous
  orders and fires nothing; on revival within the doc 07 D1 grace the pilot reclaims the seat at a pulse
  boundary with a new epoch; after the grace the seat is released and the tank idles. This substitutes
  "idle" for doc 07 §1.1's fallback controller until the reflex controller (§6) takes that role, and is
  recorded as an amendment (§11).

---

## 4. Records and definitions

**Content packs.** Game data lives in `crates/tacsim/content/` and loads through `mush-content` as
`RulesDefinition` packs with the existing latch; the centre's rooms live in the atlas-owned OOC pack:

```text
crates/mush-atlas/content/neighborhood.ooc_area.json   version 4: room budget raised to 10; the
                                                       simulator lobby, red room, blue room, their
                                                       exits, the selector item, and eight pod items
                                                       carrying `interior: true` (new item field)
crates/tacsim/content/
  units/line_tank.json            UnitSpec: sections, facets, cells, components, capabilities,
                                  mobility mode rules, facet table, silhouettes, sensor_height_cm
  weapons/cannon_105.json         delivery, mount, bands, dispersion, cycle_ms, ammunition, burst,
                                  muzzle speed, heat
  weapons/mg_coax.json
  ammunition/he_105.json          damage, penetration_class, count_per_load
  ammunition/ball_762.json
  sensors/visual_mk1.json         max_range_cells, max_attenuation, classify_after_ms, sees_wrecks,
                                  emission: null, jamming: null
  resolution.json                 hit_curve table, error_terms, p_min, p_max
  arenas/proving_ground.json      the 2.56 km arena: tiles, entry zones, key spots, waypoint graph,
                                  the eight tank objects' initial placements
  resources.json                  the ten commodity families as InstrumentDef rows (Commodity),
                                  shipped unconsumed by the story
```

Every definition is registered through the C1 registry with an exact `DefinitionRef`; a persisted unit
points at exact versions; a changed definition is a new version with a migration chain.

**Arena state** (RAM): the arena definition, per-unit `{pose, speed_cms, desired_heading,
desired_speed, power, unit: TacticalUnit, ready_at per weapon, lock}`, the epoch table, contacts per
platform, due events (a min-heap by timestamp then sequence), the spatial index, the tempo, the ledger
egress ring.

**Ledger events** (JSON lines, chunked at 16 KiB, streamed): `{seq, t_ms, kind, ...}` with kinds
`match_started, tempo, startup, position_summary (every unit, every ledger.summary_ms of arena time,
in every tempo), detected, classified, lost, locked, fired, impact, hit, miss, section_destroyed,
unit_disabled, unit_destroyed, lagged, match_ended, aborted, void`, each carrying unit ids, the draw
label and value where a draw occurred, and the substep state hash. There is no per-pulse `moved`. Bounds:
`ledger.summary_ms` (5,000), `ledger.max_bytes` (64 MiB; at 90% summaries stop, at 100% the match is
stopped with `Timeout`), `match.max_duration_ms` (3,600,000).

**The result record** (world side, one document per match, well under the 256 KiB record ceiling):
teams and pilots, outcome (`red | blue | draw | abort | timeout | void`) with reason, per-unit outcome
(`intact | disabled | destroyed | custody_lost`, shots fired, hits), the ledger's log id and digest, the
arena definition digest, start and end times.

---

## 5. Verbs and output

Cockpit output is one feed, `arena.cockpit`, carried as `Panel{feed, kind: Keyframe | Delta}` with the
map frame (terrain glyphs plus the platform's contacts), the status block (power, speed, heading, armor
and internals per section as current over max, ammunition and ready state per weapon, current lock and
its percentage), and the contacts list; plus `Notice` events for transitions and refusals. Frames reach
the world task as latest-wins slots and go out through its normal output path (§3.1). The ANSI line
renderer prints the frame as text; the TUI and web render the same structure; parity is a golden test in
each renderer. Nothing in the frame identifies another unit by object id or name before it is
classified, and a classified contact is named by class and team, never by pilot.

---

## 6. AI

**The reflex controller** (D-16-16) is a deterministic state machine per AI unit in the game crate, run
in the arena's pulse (phase 4) and submitting the same intents a pilot's verbs stage:

`Startup → Patrol (the arena pack's waypoint graph) → Approach (nearest classified hostile by a utility
score of range, facet exposure, own health) → Engage (hold the best band, lock, fire when ready) →
Withdraw (below a health threshold, toward the entry zone) → Disabled`. Reactions to `blocked`, `lost`,
`no line` and `unreachable` are edges in the machine; `unreachable` (the bounded A\* over budget or no
path) falls back to the nearest waypoint. Path queries use `[atlas].path_max_cells` raised to cover the
window and are bounded per pulse (`harness.max_path_queries_per_pulse`). It drives the harness's
non-fighting units and the AI fill of a later lobby; it is not part of the story test, and it is the
fallback controller doc 07 §1.1 asks for once it exists.

**Deliberation** is the existing durable decision pipeline, requested per squad or commander rather
than per unit, event-driven with a floor of `command.deliberate_min_ms` (30,000), returning an order
(objective, target, horizon, constraints) with a time to live through the same admission; the fallback
when nothing arrives is the reflex doctrine, not idle. Not in the story; the hook is the order type and
the squad grouping.

**Provider failure** changes nothing in the arena: the pipeline's retries, breaker and fallback never
block a pulse. The visible in-world effect ("mind link dropped, unit on doctrine") is a notice when an
order expires unrenewed.

**The cost run** (16a Q9b): a mode of the harness target gated by `TACSIM_LLM_COST_RUN=1`, the
OpenRouter key and `TACSIM_LLM_COST_CAP_USD`, never run by CI, that turns deliberation on for N minutes
of harness play and reports calls, tokens and USD per unit-minute, per squad and per match from
`mush-llm`'s budget and audit records.

---

## 7. The game crate

`ruleset-tacsim` becomes `crates/tacsim`, the game (D-16-13). It holds:

- `composition.rs`: the production composition root, moved from `mush-server/src/modules.rs` together
  with `sim_routes.rs`; `mush-server` keeps host wiring only (accounts, gateway, IPC, persistence,
  task spawning). `OnboardingConfig` moves out of the deleted WoD crate into `mush-core`. `cargo tree
  -p tacsim` is acyclic by test.
- `arena/`: the core library (`Arena`, `step`, tempo, motion, perception, resolution, damage, the reflex
  controller, the ledger, the RNG labels);
- `arena_task.rs`: the tokio wrapper, the seven seam messages, the inbox and egress ring, the watchdog;
- `cockpit/`: the verbs and the frame builder;
- `match/`: the lifecycle state machine, custody, boot reconciliation, the result record;
- `definitions/`: typed decoders for the content packs;
- `sheet.rs`: the one character sheet (pilot ratings) over `mush-ruleset`'s schema library;
- `content/` as in §4.

**Configuration** adds under `[tacsim]`: `[tacsim.arena]` (tempo periods, thresholds, holds, substep
max, stall, catch-up, intents bound, ledger bounds), `[tacsim.match]` (slots, startup, board timeout,
max duration), `[tacsim.sensors]`, `[tacsim.resolution]` (paths to the named tables),
`[tacsim.harness]` (thresholds), all `deny_unknown_fields`, mapped in `config_ext.rs`, and present in
`mushd.toml.example`, the instance template and both dev deployment files.

**What stays Rust** (D-16-13): the mutation verbs, `CellAddr` arms, instrument kinds, consequence kinds
(`destroy_unit`, `remove_weapons`, `slow`, `no_turn_toward`, `disable_motion`), trace kinds, sensor
comparison terms, order channels, tempo states, match states, the seam messages, the draw labels, the
resolution mechanic. Config picks from these by name. A config file that needs a conditional means the
vocabulary lacks a verb, and the verb is added in Rust.

---

## 8. Tests

- **A1**, the story test, in `mushd/tests` and `mush-server/tests`, with the surface-by-surface observer
  assertions of §1.
- **A2**, the harness, an ignored test in `crates/tacsim/tests/harness.rs` that drives `Arena::step`
  directly, named in `ci.yml` as `-p tacsim --test harness` (so `ci_gated_test_inventory` is satisfied),
  run report-only on `workflow_dispatch` and on a schedule per D-H-8, and asserting its thresholds under
  `TACSIM_ASSERT=1` locally or on a dedicated runner (D-16-19).
- **The cross-tempo property test**: one scripted intent and control log at 5,000, 1,000 and 500 ms
  yields one event list and one hash sequence, crossing an impassable cell and an occupied cell.
- **Anchors kept**: the M32 atomicity and refuse-not-clamp tests, the existence-hiding and
  byte-identical-refusal tests, the ground engagement composite, the doorway traces, the mobility
  virtual-switch test. `s9_fire_gate` retires with the `+fire` lane it tests (D-16-20).
- **Gates**: the panic budget, the state-slot allowlist, the legacy-handler baseline, the restriction
  lints census and the module-namespace owners are re-blessed in the commits that change them;
  `mush-contract-tests` gains one test that the arena library has no path to `ObjectStore`,
  `WriteBatch`, `GatewaySink` or `&mut World`, and one that pins the draw-label set.
- **Restart mid-match**: a test kills or restarts the server during `Running` and asserts units mutable,
  pods free, an `aborted(restart)` result.
- **The cost run** (§6).

---

## 9. Extension acceptances

Each is a test that passes without changing A1:

1. **Lasers**: a weapon row with `delivery: beam`, `flight_time = 0`, `heat_per_shot > 0`; resolution
   uses `beam_path` attenuation as a cover term; a `heat` cell and dissipation appear on the unit. W2
   holds: laser damage against `line_tank` armour is small by definition.
2. **Bipeds**: a `UnitSpec` with `left_leg`, `right_leg`, `torso`, `head`, `left_arm`, `right_arm`
   sections, a `legged` mobility mode, taller silhouettes and a taller `sensor_height_cm`, a facet
   table that sends `front` to torso and arms; §3.4 unchanged except leg consequences.
3. **Thermal sensor**: a sensor row with its own range and signature factors (an idling tank is cold);
   one new signature term; no new trace.
4. **ECM**: a component that adds to the observer's `max_attenuation` budget and lowers signature
   factors within a radius; the `jamming` field becomes non-null.
5. **ECCM**: a component that subtracts from that term.
6. **Elevation**: import one heightfield into the arena; `z_cm` follows `elev_cm`; slope enters
   `move_cost` and the trace profile enters LOS; the harness re-runs.

---

## 10. Build cuts

See [16b](16b-combat-implementation-plan.md).

---

## 11. Supersessions and amendments

- **doc 06 §6.3** "no task or channel per unit or domain": amended to "one writer per authority; a
  running arena is an authority for its own facts; the seam is the seven messages of §3.1".
- **doc 06 §6.2** `FixedStep{period_ms}`: an arena's period is adaptive (§3.1); the `mush-sim`
  `DomainClock` floor of one second is unchanged for world-side domains.
- **doc 07 §1.1**: until the reflex controller exists (16b cut N), a linkdead pilot's tank idles rather
  than falling to a `LinkLossPolicy` controller; §1.2's linkdead onset and revival events are delivered
  to the arena as `Control` messages. As built (16b cut K.2, 2026-09-17): a kernel `HookPoint::Linkdead` dispatched where the engine announces link-dead, revival carried as a second hook method rather than a widened `connected`, and the arena told `Linkdead`, `Live` with a new epoch, or `Released` past the grace. As built (16b cut N.1): the reflex controller takes the fallback role. A seat released past the linkdead grace hands the tank to the reflex controller in `Withdraw` (a tank that was still cold stays cold), the pilot cannot reclaim it, and a revival on a reflex-held tank hands it back; the controller is a unit attribute at custody (`Controller::Pilot` for a seated pilot, `Reflex` otherwise), and the lobby fields AI on unclaimed pods behind `[tacsim.match].ai_fill`, off by default until a later lobby owns the fill.
- **doc 10 §8**: read with §2's W2 and W10.
- **doc 11-h D-H-8** (the scale harness is report-only in CI): kept; the owner's preference for a
  gating harness is met by `TACSIM_ASSERT=1` locally and on a dedicated runner (D-16-19).
- **doc 14 §5.3** "what rulesets register": what the game config registers, through the C1 registry.
- **doc 04 §3** (`mush-ruleset` plugin API, one ruleset per server): retired; `mush-ruleset` is the
  platform's sheet library with one implementation.
- **`mush-sim::combat`** (`+fire` and its family, `tactical_intent` staging, the placeholder
  resolution in `tick.rs`, `s9_fire_gate`, the `+fire` allowlist stanza): retired (D-16-20). The
  `domaintick` pulse remains for world-side non-combat domains; a test pins that a deployed unit is
  never in a world-side pulse.
- **README superseded decisions**: two new entries (the single game; one writer per authority).
- **§3.3, §3.1 "Custody" and §3.8 (the tank reset and the releases)**: as built (16b cut F.1, 2026-09-19): the eight stock tanks are reset to their `UnitSpec` and returned to their authored slot at every transition of the centre to `Idle` and inside the boot pass, **not at check-in**. Check-in commits the arena's final cells and pose as the match's evidence, which the terminal centre shows until it is reset. The reset is one `UnitMutationPlan` through `mush_tactics::module::apply_plan`, keyed on the centre's roster, compare-first, and it never touches a `deployed` unit or a unit staff fielded through `@arena/start`. An abort from `Boarding` releases seats, billets, focus and bodies as well as slots and pods: one sweep, also run by `SIM LEAVE`, by the reset before its idle test, by check-in and by the boot pass, so a restart mid-match leaves nobody in a pod. A player's `SIM TEAM` on a terminal centre performs the reset and then claims, so two players rematch with no staff; `@arena/reset` remains. A unit enters an arena on what its own record says (a wreck, a lost track, the rounds it has left), and a destroyed unit checks in its true ammunition.
- **§3.7 `EXIT`**: as built (F.1): a `Released` seat (by `EXIT`, a `QUIT` or the linkdead grace running out) drops its pilot from the arena's frame roster and from the registry binding, and the board shows that slot as `left`; the tank stays with the reflex controller for the rest of the match (the N.1 amendment above), and the texts say so.
- **§5 and §3.4 step 4 (what a pilot is told)**: as built (F.2): notices ride the path of the contact notices, as `Sys` lines tagged `arena`: the shooter's hit, miss with the ledger's reason, and kill, by contact id only; the victim's hit with the section and facet struck and what is left there, and a destroyed section with the consequences that fired, never anything of the shooter; every arena-side refusal of `STARTUP`, `LOCK` or `FIRE` as `{ORDER} refused.` plus the cockpit's own sentence, a purged intent staying silent; one `Held:` line per stop with its cause. The frame carries the status block's sections (condition, internals and facet armour as current over max) and `STATUS` prints it. `section_destroyed` is journalled on the transition only. The hit percentage is per weapon, and the arena quotes the chance of the weapon a bare `FIRE` fires when the lock resolves.
- **§3.6 (what a platform knows)**: as built (F.3): a contact records this platform's own last sighting and the plot (position, heading, speed) of the sighting that last refreshed it. The mesh carries only a classified contact the sharer's own sensors saw inside the time to live, hands over that sighting's stamp and plot, and never raises a receiver's stamp above it: no second hop, and no contact kept alive by two teammates refreshing each other. Frames, `CONTACTS`, `VIEW`, both tempo signals, the reflex controller, the squad brief and the advertised hit chance read the plot; `FIRE` and the impact read true positions. Every unit has a sensor pass, because the pass is where contacts expire. `CONTACTS` carries a range in metres rounded to 50 beside the band word, which is read against the fired weapon's authored bands (a playtest decision for the owner to revisit; on the shipped content only `close` and `effective` can be printed).
- **Accounts and the layer switch** (doc 10 R3, §3.8 "one slot per account"): as built (F.4): the account oracle is one shared mutable map; `ServiceReq::CreatePlayer` carries the account (`IPC_VERSION` 6) and the mapping is made in the turn that creates the character and removed when the character is destroyed or its link fails, so a character made after boot can claim a slot with no restart. With `[tacsim].enabled` off, every `SIM` selector and `BOARD` refuse with one sentence naming the layer. The centre's gates stand aside outside the centre's rooms and pods. The proving ground musters on the ford road (arena pack version 2) and the OOC pack is at version 5.
- **Before IC play** (additions, not changes): per-pulse state durability on an arena writer lane with
  disjoint key ownership; resume from snapshot plus ledger tail; the 10 km arena; AI takeover of
  dropped human slots.

---

## 12. Decisions (owner, 2026-09-16, binding)

| # | Decision | Source |
|---|---|---|
| D-16-1 | One arena task per map; the arena is the single writer for its facts while a match runs; the seam is the seven messages of §3.1; never a shared lock | 16a Q1 B, review |
| D-16-2 | Adaptive tempo per arena: `Quiet` 5,000 ms, `Alert` 1,000 ms, `Engaged` 500 ms; the model is rate-independent through substeps and timestamped events | 16a Q2 B |
| D-16-3 | OOC matches hold state in RAM and the arena never touches the store; the ledger is streamed through the world task; a crash voids the match; per-pulse durability and resume are requirements before IC play | 16a Q3 C, Q4 A, review |
| D-16-4 | Integer authoritative state; integer `Pose`; draws derived from `(seed, label)`; a state hash per substep | 16a Q5 A, Q10 B, review |
| D-16-5 | One 2.56 km arena at 10 m cells; the 10 km path is an addition | 16a Q11 A |
| D-16-6 | M32 is the unit truth; persistent per-pod tanks reset by plan; a wreck is a named condition plus revoked capabilities; hit location by facet table | 16a Q14 A, Q15 A, review |
| D-16-7 | Kinematic motion in one-cell substeps; a uniform-grid spatial index | 16a Q13 A, Q16 A, review |
| D-16-8 | Scheduled impacts; per-mille resolution from dispersion against silhouette with cover, motion and gunnery terms; fixed damage per ammunition; bursts as one binomial draw; the percentage is shown; a blocked shot costs like any shot | 16a Q17 A, Q18 A, Q33, review |
| D-16-9 | Armour to floor, overflow to internals, section consequences from a closed vocabulary; disabled precedes destroyed | 16a Q19 A, Q20 A |
| D-16-10 | Detection is range plus LOS plus signature with a classification delay; arena contacts are an in-RAM shadow with their own id space; the arena's domain is excluded from every world view; ECM and ECCM are null fields today | 16a Q21 to Q24, review |
| D-16-11 | Pods are interiors; stations on the tank; F3 seats and focus; bare cockpit verbs and `BOARD`; a ten-second startup | 16a Q25 to Q27, review |
| D-16-12 | The minimal lifecycle with a named `Boarding → Running` trigger and `Aborted | Void` from every allocated state; doc 07 §1 with the idle substitution | 16a Q28, Q29, review |
| D-16-13 | `crates/tacsim` is the game and the composition root; definitions through the C1 registry; the closed vocabularies stay Rust; configuration plumbed end to end | 16a Q29 to Q32, review |
| D-16-14 | The character rulesets are deleted first; `mush-ruleset` stays as the sheet library with one game sheet | 16a Q34 A, Q35 A |
| D-16-15 | The story test and the harness are the acceptances; gates are never left red; the cost run is owner-directed | 16a Q7 to Q9, Q36 |
| D-16-16 | Reflex controller in the game crate; deliberation through the decision pipeline per squad; provider failure is invisible to the pulse | 16a Q37 to Q39 |
| D-16-17 | W1 to W16 are ratified world facts | 16a §9 |
| D-16-18 | Extension order: lasers, bipeds, thermal, ECM, ECCM, elevation | 16a Q40 |
| D-16-19 | The harness follows D-H-8 in CI (report-only, on demand and scheduled) and asserts under `TACSIM_ASSERT=1` locally or on a dedicated runner; an owner wish to gate CI on it supersedes D-H-8 and is recorded here if given | review |
| D-16-20 | The shipped `mush-sim` combat verb lane is retired; the arena is the only combat lane | review |
