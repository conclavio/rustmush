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
test in `mushd/tests` (in-process) and its twin in `mush-gateway/tests/arena_story_split.rs` through the gateway boundary; both must
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

**The seam is eight messages and nothing else.**

```text
world -> arena   CustodyOut { definition, units[{cells, pose}], pilots[{unit, station, epoch}], seed }
world -> arena   Intent     { unit, channel, epoch, kind, seq, payload }        (bounded inbox)
world -> arena   Control    { unit, channel, epoch, state: Live | Linkdead | Released }
world -> arena   Stop       { reason: Abort | Timeout | Void | Concede{by} | Unattended }   (16b cut D.3)
arena -> world   Ledger     { chunk }                                            (bounded egress)
arena -> world   Frames     { now_ms, pilot -> frame }                           (latest-wins per pilot)
arena -> world   CustodyIn  { units[{cells, final_pose, outcome}], result, ledger_digest, reason }
arena -> world   Deliberate { match_id, signals: squad briefs }                  (droppable, N.2)
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
  frames, never events. Since 16b cut D.1 the message also carries the arena's clock and may carry no
  frame at all: it is sent when a frame exists or the arena second has moved since the last one sent,
  so the lobby board's clock is the arena's own and at worst an arena second stale (§11).
- `CustodyIn` is applied per unit and per-unit fallible (§Custody).
- `Deliberate` is a report-only squad signal; the world may request a bounded LLM
  completion and return an ordinary `Intent`. The arena never waits for it.
  These are eight conceptual seam messages. In `arena_task.rs`, `CustodyOut` is
  the spawn argument and `CustodyIn` rides `Ended`; `LedgerAck` and `Lagged` also
  carry reliability bookkeeping, making nine runtime enum variants in total.

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
| `Engaged` | 500 ms | a shot is fired, an impact is scheduled, or a platform holds a classified hostile it has not read dead (§6; 16b cut E.2) inside its inner fighting reach, the lesser of its own sight and its loaded weapon's first band (16b cut E.1, §11) | `tempo.engaged_hold_ms` (10,000) since the last shot, impact or in-range pair |

Tempo changes are ledger events. The harness's 25 ms p99 applies in `Engaged`.

**Rate independence** (D-16-2). The tempo decides only how often `step` runs and how output is batched;
it never decides an outcome. That is achieved by one rule: **a pulse is a sequence of substeps, and
every due event is applied at its own arena timestamp between substeps.**

- The substep length is `min(tempo.substep_max_ms, (cell_cm - 1) × 1000 / max_speed_cms_of_any_unit)`,
  floored at 50 ms, so no unit ever advances more than one cell in a substep (§3.4). The centimetre
  taken off the cell is margin (16b cut Q.6, §11): a substep's travel, its carried remainder included,
  is at most `cell_cm - 1`.
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
   discrete actions are queued per unit, bounded at `intents.max_pending_per_unit`. An order refused
   here as a wreck's, an immobile tank's, one past the bound or one given with power off is told to its
   pilot; a purged one stays silent (16b cut D.2, §11).
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
watchdog, the pulse that ended the match included: the task reads the watchdog before it honours the end,
returns no units, and the black box keeps the arena's own end (16b cut AA.3, §11).

### 3.2 Space

**Pose is integer** (D-16-4): `Pose { x_cm: i64, y_cm: i64, z_cm: i32, heading_mdeg: u32, posture }`
with `heading_mdeg` in `0..360_000` clockwise from north. The `@spatial/place` wire keys become
`x_cm, y_cm, z_cm, heading_mdeg`; the legacy pose wire arm is deleted. Authored packs keep metres in JSON
and convert at load. `z_cm` is carried from the first build and equals the cell's `elev_cm` (the arena
makes it so at check-out; see §11 for elevation as built).

**Range** is one function used everywhere: Euclidean centimetres between two poses. Range bands,
angular size, flight time and the tempo trigger use it; the atlas `Trace` supplies cover and blocking
only, never distance (its `range_cells` is Chebyshev).

**The arena window** is one `mush-sim` domain over one region surface at `cell_cm = 1000`, 256 by 256
cells (D-16-5): 2.56 km square, the current `MAX_TILE_SIDE`. The path to 10 km is a per-driver tile side
and a tiled composite, specified in §11 as an addition and re-run through the harness first.

**Terrain** is the existing 18-class cell grid with the engine's cover and opacity per class. The first
arena is authored with roads, open ground, light and heavy woods, shallow water, a few buildings, two
team entry zones, three or four named key spots, and a waypoint graph for the reflex controller (§6).
The proving ground is surveyed ground: its terrain is known to every pilot at check-out, its units are
not (16b cut Q.3, §11).

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
that. A wreck is an obstacle and a contact, never a target of a lock, a shot or an impact, which read the
physics; whether a CONTROLLER still treats a held contact as a target is knowledge (§6; 16b cut E.2,
§11): a machine keeps a target it has not read dead until the contact expires.

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
4. If a cell boundary is crossed, the cell entered is tested: impassable for `tracked`, too steep, or off
   the window stops the unit at the boundary with `speed = 0` and one `blocked` notice per stop;
   occupied by another unit holds the unit at the boundary keeping its speed, silently, for
   `[tacsim.arena].occupied_stop_ms` (2,000) of arena time and then stops it the same way (16b cut E.3,
   §11). **Invariant: no unit ever occupies a cell that was not tested on entry.**
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
arena cannot write. A pilot neither sees nor types this id (16b cut Q.1, §11): a contact is shown and
typed as a short designator, a provisional viewer-scoped `X<n>` until it is classified and then the team
letter and the slot letter, and the id stays what the arena receives, so a lock survives the rename.
Any name a pilot types that is unresolvable, unowned or expired gets one byte-identical refusal.
Contacts die with the match; the ledger's `detected`, `classified` and `lost` events are the record.

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

**What a pilot sees**: `CONTACTS` lists the platform's contacts with designator (§11, cut Q.1), class or
`unknown`, bearing, range band, age; notices fire once per transition (`acquired`, `classified`, `lost`)
with hysteresis. A loss cancelled inside the hysteresis window is silent unless the row comes back
`unknown` under a different name: then the regression is said once (`unidentified`: `BA is unidentified
again: X2.`) and the old name is never accepted as an alias. The platform is also told when its own
receiver becomes degraded by jamming and when it clears (`receiver_degraded`, `receiver_clear`), with no
source, bearing, team or id (16b cut D.4, §11). The band word is scope advice, read against the lesser of
the platform's own sight and the fired weapon's last band (16b cut D.1, §11). `VIEW` renders the arena window from terrain plus the platform's own contacts; `SCAN`
reads an identified contact's sections as green, yellow, red or destroyed from a close look, never as a
number (§11, cut Q.2). Teammates appear when shared. The ground is surveyed and the units on it are not
(§3.2). Nothing else about another unit crosses to a pilot before detection, on any
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
| `SPEED <kph>` | minus the reverse ceiling to max; a negative speed reverses (16b cut D.3) | power not online; past either ceiling |
| `VIEW` | none | none |
| `CONTACTS` | none | none |
| `LOCK <contact>` | a designator, `X<n>` or team and slot letters, case-folded (16b cut Q.1) | one byte-identical refusal for any name the platform does not hold |
| `FIRE [<weapon>]` | a weapon id, its short name (`mg`) or a unique prefix, case-folded (16b cut D.2); default cannon | no lock, cycling, empty, power not online; a word two weapons answer to; each reads `FIRE <weapon> refused. …` |
| `WEAPONSTATUS` | none | none |
| `STATUS` | none | none |
| `RANGE [<contact>]` | a designator; default the lock (Q.1) | neither a lock nor a name (its own line, not `FIRE`'s); the one byte-identical refusal for a name the platform does not hold |
| `BEARING [<contact>]` | a designator; default the lock (Q.1) | as `RANGE` |
| `SCAN [<contact>]` | a designator; default the lock (Q.2) | as `RANGE`; the contact is not identified yet; no reading yet, close the range for a look |
| `EXIT` | none | none |

`RANGE`, `BEARING` and `SCAN`, like the four read verbs before them, stage no intent: they answer from
the pilot's last frame (§11). Since 16b cut D.2 every ordering verb (`STARTUP`, `HEADING`, `SPEED`,
`LOCK`, `FIRE`) is refused locally on a destroyed tank, and the two motion verbs on one that cannot
move, off the frame's `wreck` and `immobile` keys; since D.1 `STARTUP` answers from the mirror when
power is already online or coming up (§11).

Lobby and team-room verbs: `SIM TEAM red|blue`, `SIM LEAVE`, `SIM START`, `SIM ABORT`, `SIM STATUS`
(while a match runs a player reads their own side in full and the other side as index, callsign and
pilot name alone; 16b cut D.1), `SIM AI [<n>]` (the size of the AI fill; a side's lead or staff, during
`Selection`, on a server with `[tacsim.match].ai_fill` on; 16b cut Q.7), `SIM UNIT [<type>]` (what the
caller's own slot will field, during `Selection`; Q.8), `SIM CONCEDE` (a side's lead, while the match
runs; 16b cut D.3), `BOARD`. Bare `SIM AI` and bare `SIM UNIT` are reads and answer anybody in the
centre in any state (D.1). Staff have `@arena/status|start|intent|abort|reset|release` (the last since
D.3). The shipped `mush-sim` combat verbs (`+fire +launch +heading +thrust
+posture +dc +dock +eject`), their `tactical_intent` staging and the placeholder resolution are retired with this build
(D-16-20); ships and other domains later get verbs on the same arena model.

### 3.8 Lifecycle

```text
Idle → Selection → Boarding → Running → Ended → Idle
                 ↘        ↘        ↘
                  Aborted ← Aborted ← Aborted | Void → Idle
```

- `Selection`: `SIM TEAM` claims a slot; one slot per account; one to `match.max_slots_per_side` per
  side; no budgets, terms or consent (16a Q28). `SIM UNIT` chooses what the caller's own slot fields and
  `SIM AI` sizes the fill (§3.7).
- `Boarding`: `SIM START` (a team lead or staff). With `[tacsim.match].ai_fill` on it needs one claimed
  slot in the whole centre, and the machines take the unclaimed pods of both sides (16b cut Q.7, §11);
  with the fill off it needs at least one pilot per side. After every refusal and before a pod locks,
  every pod's tank is fitted to its slot's chosen type or else its authored one (Q.8, §11); pods lock to
  their pilots; pilots `BOARD`. The transition `Boarding → Running` fires when every claimed slot is
  seated or `match.board_timeout_ms` elapses with unseated slots dropped (with the fill off an empty side
  aborts; with it on only an empty centre does; a dropped lead, or a lead who leaves the tank mid-match, hands the side to its first remaining
  claimant who has not left, as `SIM LEAVE` does: 16b cuts AA.2 and B.4, §11). Check-out happens on that transition.
- `Running`: the arena steps. Team elimination (every unit of a side a wreck or disabled with no weapons)
  ends the match; both sides running dry with nothing in flight ends it as a draw, reason `spent` (Q.7,
  §11); `match.max_duration_ms` ends it as a timeout; `SIM ABORT` by any participant or staff aborts it;
  `SIM CONCEDE` by a side's lead ends it as a recorded win for the other side, reason `conceded`; a match
  whose every pilot has left ends by itself `match.unattended_ms` (300,000) later, outcome `timeout`,
  reason `unattended` (both 16b cut D.3, §11); a panic or ledger overflow voids it.
- `Ended | Aborted | Void`: check-in (§3.1); result written; tanks reset; seats, leases, focus and pods
  released atomically; the reset is idempotent and re-runnable by staff. An abort from `Selection` or
  `Boarding` has no custody to return: it releases slots and pods and, since D.3, puts the stock tanks
  back, through the one funnel every end before `Running` takes, a refused `Boarding → Running`
  transition included. In `Selection` `SIM ABORT` is staff's alone; `SIM LEAVE` is a player's way out.
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
`match_started, tempo, startup, power_online (16b cut Q.1: at the startup due event's own timestamp),
position_summary (every unit, every ledger.summary_ms of arena time,
in every tempo), detected, classified, lost, locked, fired, impact, hit, miss, section_destroyed,
unit_disabled, unit_destroyed, receiver (16b cut D.4: the unit and a degraded flag, one line per
transition of a platform's own receiver between clear and jammed, none for a change of degree), lagged,
match_ended, aborted, void`, each carrying unit ids, the draw
label and value where a draw occurred, and the substep state hash. `section_destroyed` and
`unit_destroyed` carry `by`, the unit whose shot did it, left off when nothing shot it (D.4). There is no per-pulse `moved`. Bounds:
`ledger.summary_ms` (5,000), `ledger.max_bytes` (64 MiB; at 90% summaries stop, at 100% the match is
stopped with `Timeout`), `match.max_duration_ms` (3,600,000).

**The result record** (world side, one document per match, well under the 256 KiB record ceiling):
teams and pilots, outcome (`red | blue | draw | abort | timeout | void`) with reason, per-unit outcome
(`intact | disabled | destroyed | custody_lost`, shots fired, hits), the ledger's log id and digest, the
arena definition digest, start and end times. The six outcome words are closed; a match that ends
because both sides ran dry is a `draw` whose reason is `spent` (16b cut Q.7, §11), a conceded match is the
other side's win with reason `conceded`, and a match every pilot left is a `timeout` with reason
`unattended` (16b cut D.3, §11). A unit's `hits` counts
the rounds that took something off something, which is the black box's `hit` lines less the rounds
that landed on an already spent section (Q.6).

---

## 5. Verbs and output

Cockpit output is two feeds (16b cut Q.3, §11). `arena.cockpit` is pushed, carried as
`Panel{feed, kind: Keyframe}` with the map frame (terrain glyphs plus the platform's contacts),
the status block (power, speed, heading, armor and internals per section as current over max, ammunition
and ready state per weapon, current lock and its percentage), and the contacts list; plus `Notice`
events for transitions and refusals. `arena.map` is the whole arena's terrain and nothing about any
unit: surveyed ground, pulled by a panel client through a query and never pushed, answered only to a
seated pilot and only for their own match. Frames reach the world task as latest-wins slots and go out
through its normal output path (§3.1). The ANSI line renderer prints the frame as text; the TUI and the
web client render the same structure on their screen 0 under one map-window rule; parity is one
canonical fixture set whose line-rendered grid is the byte target for all three renderers. Nothing in
the frame identifies another unit by object id or name before it is classified; until then it is a
provisional `X<n>` that says nothing of team or slot. A classified contact is named by class, team and
its slot designator. The slot designator is public lobby knowledge, so the lobby board names the pilot
behind it; the owner accepted that, and the board is not blanked (Q.1, §11).

---

## 6. AI

**The reflex controller** (D-16-16) is a deterministic state machine per AI unit in the game crate, run
in the arena's pulse (phase 4) and submitting the same intents a pilot's verbs stage:

`Startup → Patrol (a movement order, else the search, else the hunt, else the arena pack's waypoint
graph; 16b cut E.4) → Approach (nearest classified hostile by a utility score of range against the
platform's own fighting reach, facet exposure, own health; it closes to `engage_band_permille` of the
lesser of its sight and its loaded weapon's inner band, 420 m on the stock tank, or on the plot's own
cell when it has no line; a machine that cannot close, its engine dead or its hull authored with no
speed, enters Engage anywhere inside that lesser reach, 600 m: 16b cut AA.1) → Engage (stop, turn, lock,
fire when ready out to that lesser reach, and back to Approach past it, 600 m) → Withdraw (below a
health threshold, toward the entry zone; on the way it
turns only on what is inside the close band and while it has `stand_min_permille` of its plate, and once
home, or when it cannot close, it shoots back at anything inside its release reach) → Disabled` (the
ranges: 16b cut E.1, §11; the immobile machine: AA.1, §11).
Reactions to `blocked`, `lost`, `no line` and `unreachable` are edges in the machine; `unreachable` (the
bounded A\* over budget or no path) falls back to the nearest waypoint, and when the cell that failed was
a live `Advance` order's own it ends that order for the unit (16b cut E.2); an `Advance` held by a unit
that cannot close ends for that unit on its next decide tick the same way, while a `Hold` holds it as it
holds any machine (AA.1). `lost` is read off the
contact: a target is dead when the platform's own gunner killed it or its held reading shows a fatal
section destroyed, and is otherwise held until the contact expires (E.2). Path queries use `[atlas].path_max_cells` raised to cover the
window and are bounded per pulse (`harness.max_path_queries_per_pulse`). It drives the harness's
non-fighting units and the lobby's configurable AI fill (Q.7); it is not part of the two-human
story test, and it is the fallback controller for released or disconnected pilots (N.1). As built (16b cut Q.7, §11): a stop is learned
at the journalled `blocked` event, `Withdraw` is not terminal (a cornered unit stands and shoots inside
the bands above, and a unit home and quiet rallies to `Patrol` once, never while an `Objective::Withdraw`
order stands: 16b cuts E.1 and E.2), and a unit that has run dry withdraws for good.

**Deliberation** (as built in N.2) uses `LlmService::complete` off the world loop,
requested per squad, with a floor of `arena.deliberate_min_ms` (30,000), a bounded
order (objective, target, horizon, constraints), a time to live and doctrine fallback.
An omitted time to live is the floor, and the horizon defaults to it (16b cut E.2).
The typed reply returns through normal intent admission. It does not use K7's durable
decision pipeline: matches end on process restart, and there is no surviving match
into which an old squad order could recover. Moving this onto K7 belongs with match
resumption; the tradeoff is recorded in `tacsim/src/deliberation/mod.rs`.

**Provider failure** never blocks a pulse. `mush-llm` bounds requests, retries and
budgets; a missing or invalid reply leaves an existing order active until its TTL
expires, then the reflex controller falls back to doctrine. An order that
expires unrenewed produces the in-world notice that the mind link has dropped.
An `Advance` order whose cell no route reaches also ends, for that unit, at the
tick its search failed, through the same `order_expired` ledger line and the squad's next signal (16b cut
E.2), and so does an `Advance` held by a unit that cannot close, on its next decide tick (AA.1); the
signal carries no reason and no mobility fact, and the pilots' notice still follows the order's nominal
time to live, so that end is announced late or, when the commander has replaced the order first, not at
all (a 16c candidate).

**The cost run** (16a Q9b): a mode of the harness target gated by `TACSIM_LLM_COST_RUN=1`, the
OpenRouter key and `TACSIM_LLM_COST_CAP_USD`, never run against a live provider by CI, that turns deliberation on for N minutes
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
- `arena_task.rs`: the tokio wrapper, the eight seam messages, the inbox and egress ring, the watchdog;
- `cockpit/`: the verbs and the frame builder;
- `match/`: the lifecycle state machine, custody, boot reconciliation, the result record;
- `definitions/`: typed decoders for the content packs;
- `sheet.rs`: the one character sheet (pilot ratings) over `mush-ruleset`'s schema library;
- `content/` as in §4.

**Configuration** adds under `[tacsim]`: `[tacsim.arena]` (tempo periods, thresholds, holds, substep
max, stall, catch-up, intents bound, ledger bounds), `[tacsim.match]` (slots, startup, board timeout,
max duration, and since 16b cut D.3 `unattended_ms`), `[tacsim.sensors]`, `[tacsim.resolution]` (paths to the named tables),
`[tacsim.harness]` (thresholds), all `deny_unknown_fields`, mapped in `config_ext.rs`, and present in
`mushd.toml.example`, the instance template and both dev deployment files.

**What stays Rust** (D-16-13): the mutation verbs, `CellAddr` arms, instrument kinds, consequence kinds
(`destroy_unit`, `remove_weapons`, `slow`, `no_turn_toward`, `disable_motion`), trace kinds, sensor
comparison terms, order channels, tempo states, match states, the seam messages, the draw labels, the
resolution mechanic. Config picks from these by name. A config file that needs a conditional means the
vocabulary lacks a verb, and the verb is added in Rust.

---

## 8. Tests

- **A1**, the story test, in `mushd/tests/arena_story.rs` and `mush-gateway/tests/arena_story_split.rs`, with the surface-by-surface observer
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
  running arena is an authority for its own facts; the seam is the eight messages of §3.1".
- **doc 06 §6.2** `FixedStep{period_ms}`: an arena's period is adaptive (§3.1); the `mush-sim`
  `DomainClock` floor of one second is unchanged for world-side domains.
- **doc 07 §1.1**: until the reflex controller exists (16b cut N), a linkdead pilot's tank idles rather
  than falling to a `LinkLossPolicy` controller; §1.2's linkdead onset and revival events are delivered
  to the arena as `Control` messages. As built (16b cut K.2, 2026-09-17): a kernel `HookPoint::Linkdead` dispatched where the engine announces link-dead, revival carried as a second hook method rather than a widened `connected`, and the arena told `Linkdead`, `Live` with a new epoch, or `Released` past the grace. As built (16b cut N.1): the reflex controller takes the fallback role. A seat released past the linkdead grace hands the tank to the reflex controller in `Withdraw` (a tank that was still cold stays cold; since 16b cut D.3 it orders its own startup and drives home still in `Withdraw`: the D.3 entry below), the pilot cannot reclaim it, and a revival on a reflex-held tank hands it back; the controller is a unit attribute at custody (`Controller::Pilot` for a seated pilot, `Reflex` otherwise), and the lobby fields AI on unclaimed pods behind `[tacsim.match].ai_fill`, off by default until a later lobby owns the fill. As built (16b cut Q.7): the lobby owns it; the fill is sized, one pilot alone can start against it, and the released tank's `Withdraw` stands and rallies (the Q.7 entry below).
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
- **§5 and §3.4 step 4 (what a pilot is told)**: as built (F.2): notices ride the path of the contact notices, as `Sys` lines tagged `arena` (since 16b cut B.1 the per-pulse copy of a frame notice carries the span tag `arena-notice` beside it, so a cockpit ring that composes the notice from the frame never rings it twice; the replies keep `arena`): the shooter's hit, miss with the ledger's reason, and kill, by contact id only (since 16b cut Q.1 by the contact's designator, and since Q.2 the hit names the section struck and a destroyed section is one more line, both for an identified contact only: the Q.1 and Q.2 entries below); the victim's hit with the section and facet struck and what is left there, and a destroyed section with the consequences that fired, never anything of the shooter; every arena-side refusal of `STARTUP`, `LOCK` or `FIRE` as `{ORDER} refused.` plus the cockpit's own sentence, a purged intent staying silent (since 16b cut D.2 the pulse's phase 2 tells its refusals too, `HEADING` and `SPEED` included, and every `FIRE` refusal reads `FIRE <weapon> refused.`: the D.2 entry below); one `Held:` line per stop with its cause. The frame carries the status block's sections (condition, internals and facet armour as current over max) and `STATUS` prints it. `section_destroyed` is journalled on the transition only. The hit percentage is per weapon, and the arena quotes the chance of the weapon a bare `FIRE` fires when the lock resolves.
- **§3.6 (what a platform knows)**: as built (F.3): a contact records this platform's own last sighting and the plot (position, heading, speed) of the sighting that last refreshed it. The mesh carries only a classified contact the sharer's own sensors saw inside the time to live, hands over that sighting's stamp and plot, and never raises a receiver's stamp above it: no second hop, and no contact kept alive by two teammates refreshing each other. Frames, `CONTACTS`, `VIEW`, both tempo signals, the reflex controller, the squad brief and the advertised hit chance read the plot; `FIRE` and the impact read true positions. Every unit has a sensor pass, because the pass is where contacts expire. `CONTACTS` carries a range in metres rounded to 50 beside the band word, which is read against the fired weapon's authored bands (a playtest decision for the owner to revisit; on the shipped content only `close` and `effective` can be printed; since 16b cut D.1 the pilot's word is read against the lesser of the platform's own sight and that last band, `far` prints, and the commander's word was a different distance until 16c `T-81` closed at 16b cut E.1: the D.1 and E.1 entries below).
- **Accounts and the layer switch** (doc 10 R3, §3.8 "one slot per account"): as built (F.4): the account oracle is one shared mutable map; `ServiceReq::CreatePlayer` carries the account (`IPC_VERSION` 6) and the mapping is made in the turn that creates the character and removed when the character is destroyed or its link fails, so a character made after boot can claim a slot with no restart. With `[tacsim].enabled` off, every `SIM` selector and `BOARD` refuse with one sentence naming the layer. The centre's gates stand aside outside the centre's rooms and pods. The proving ground musters on the ford road (arena pack version 2) and the OOC pack is at version 5 (version 6 since 16b cut D.5, with the staff lobby pack at version 4: the D.5 entry below).
- **§3.6 "ECM and ECCM are null fields today" (D-16-10)**: as built (16b cut X.4 and X.5), the two hooks are read. `jamming` is the ECM hook: `SensorDef.jamming: Option<JammingDef { radius_cells, attenuation_cost, signature_permille }>` on a sensor row, fitted to a unit through `ComponentRole::Ecm { sensor }`; the arena folds, once per sensor pass and only for a field that carries a pod, the live and powered pods within their radius of the OBSERVER into a cost taken off the observer's `max_attenuation` budget and a per-mille taken off every candidate's reach (a pod jams its own carrier; at X.4 the share mesh was not jammed, the one W6 clause then open; since 16b cut Q.6 it is, and W6 is closed: the Q.6 entry below). `emission` is the ECCM hook: `SensorDef.emission: Option<EmissionHook::Eccm(EccmDef { attenuation_recovery, signature_recovery_permille })>` on a sensor row, fitted through `ComponentRole::Eccm { sensor }`; a filter the observer carries (alive, needing no power) recovers that much of the cost and the per-mille before either is applied, never past an unjammed pass. §9 item 4's "adds to the observer's `max_attenuation` budget" is read as "adds to what is taken off it". Neither field moves a shipped digest: the stock sight writes both as null. The extension content lives in per-item tables (`content::EXTENSIONS*`) loaded through the operator route rather than the shipped catalog (since 16b cut Q.8 the production catalog is the fielded one and carries them: the Q.8 entry below), and X.2 added a sixth consequence kind, `disable_motion_when_all_of`, to §7's vocabulary.
- **§3.2 "`z_cm` … zero on the first arena", §3.4 step 2 and §9 item 6 (elevation)**: as built (16b cut X.6, 2026-09-19): the first arena is not flat (the proving ground's relief runs from -400 to +400 cm) and line of sight has honoured it since cuts S.1 and W through the atlas walk, so item 6's LOS half needed no code and partial defilade is not adopted. `Arena::new` sets every unit's `z_cm` to its cell's `elev_cm` at check-out (the arena does not trust an inbound z), and a check-in does not treat a z-only difference as a move; the world's stored z between matches is not maintained. One slope rule (`arena::slope`, over `mush_atlas::physics::slope_pct` and the shipped locomotion table) is shared by motion and the reflex A*: step 2's terrain factor is `100 / (move_cost × (100 + grade) / 100)` with `grade` the uphill slope of the edge by which the unit entered its cell (hashed as `grade_pct`); an edge steeper than the unit's locomotion limit **either way** is refused like any other blocked step, with its own cause and `Held:` line, and is not an edge to the A*, which also applies a shoulder rule on diagonals. The limit is symmetric where the atlas's is uphill-only, so no heightfield can hold a cell a unit may enter and never leave; the atlas's kerb (climb) branch is not applied at 10 m cells. A unit with no declared locomotion has no limit and still pays the surcharge. On the shipped proving ground the steepest edge between passable cells is 21 per cent against the tracked limit of 45, so nothing there can be slope-refused and a tank loses about a metre a contour.
- **§3.6, §3.7 and §5 (what a contact is called)**: as built (16b cut Q.1, 2026-09-19, and the owner's ruling of 2026-09-20): a contact is shown and typed as a short designator and never as its id. Until this platform's LIVE contact row reads `classified` it is a provisional `X<n>`, numbered per viewer in the order that viewer's OWN sensors first found things, never reused and kept across a loss (a mesh arrival consumes no number); once classified it is the team letter and the slot letter, the centre slot in bijective base 26 (`RA`, `BA`, slot 27 `RAA`), the same for every viewer, with one `Contact X2 identified: BA, blue.` line at the rename. The name is read off the live class and the per-viewer book holds numbers and nothing else, because a contact is not durable (expiry removes the row and the next detection re-inserts it `unknown` under the same id): a lost and re-detected contact is `X<n>` again until it is classified a second time, where a sticky flag would have named a re-acquired blip by team and slot. The book is folded into the state hash; a unit's callsign is not. A unit fielded with no slot (staff `@arena/start`, the harness) takes the lowest free slot of its team in unit-id order, and a repeated slot on one side refuses the arena. **A lock is on the contact, not on its name**: the arena still receives only a contact id, so a lock taken on `X1` continues when the contact becomes `BA` (the owner's ruling, which needed no code and is pinned at arena, cockpit and SSH level), and the `X<n>` alias keeps answering while the row reads as a slot; `LOCK`, `RANGE`, `BEARING` and `SCAN` take either spelling, case-folded. The sixteen digits are no longer typable: every miss is the one byte-identical `No such contact.`, so `LOCK BA` against an unheld, unknown or re-acquired-unknown `BA` cannot be told from `LOCK ZZ`. The deliberation brief keeps the opaque id as its own handle. `[tacsim.sensors].designate_on_detection` (off) names by team and slot at detection, issues no number and says no `identified` line. What it costs: W5 stops gating identification for a human (the team is readable off the name `classify_after_ms` early) and §5's rule is broken on four surfaces (the acquired notice, `CONTACTS`, the map label, the panel list); classification then gates only the mesh, the reflex target choice, the glyph, the section words and `SCAN`. A slot callsign plus the public lobby board (`SIM STATUS` prints `1 (RA): ...`, and `STATUS` opens with the pilot's own callsign) names the pilot behind a classified contact: the owner accepted that, and the board is not blanked.
- **§3.6, §3.7 and §4 (`RANGE`, `BEARING`, the lock step, power online)**: as built (Q.1): `RANGE [<contact>]` and `BEARING [<contact>]` read the F.3 plot of the named contact, else of the lock, and add `, plotted N s ago` when the plot is older than `sensors.pass_ms`. With neither a name nor a lock they refuse with their own line (`FIRE` keeps its own), and a name the platform does not hold takes the one `No such contact.` even from a pilot who has no frame yet, so a name can never be told from a missing scope. A LOCKED contact's range is reported to 10 m and every other contact's to 50 m (F.3's step; since 16b cut D.4 the locked step is derived from the plot's age, 10 m on a fresh plot widening to 50 m: the D.4 entry below). Since 16b cut B.4 an unlocked contact's figure is held against the one the pilot was last sent while the true range stays within five eighths of the step of it (6.25 m past a 50 m edge), so a range at an edge does not flip each pulse, and `RANGE` prints the held figure for an unlocked row so it agrees with `CONTACTS` and both panels; the lock keeps its age-derived step and no memory (the B.4 entry below). `power_online` is a ledger kind, journalled at the startup due event's own timestamp on the real `starting` to `online` flip, and that tank's own pilots are told `Power online.`.
- **§3.6, §3.7 and §5 (what the gunner sees, and `SCAN`)**: as built (Q.2): the shooter's hit names the section struck (`Hit on BA: turret.`) and a destroyed section of the target is one line to the shooter (`BA's turret is destroyed.`, notice kind `target_section_lost`), both only for a contact the platform holds AND has classified at staging, and never a facet, a number or a consequence list: section ids are a chassis vocabulary, so a section word on a blip would be identification by gunfire (`Hit on X2.`). `SCAN [<contact>]` reads identified contacts only and prints each section as `green`, `yellow`, `red` or `destroyed`, never a number, under the look's own age (`Scan of BA, read 2 s ago:`). What it prints is a Reading that rides the contact: a sensor pass takes one only for a classified contact inside the close-look reach, `condition_reach_permille` (500) of the effective reach the range test just used (one comparison, no new trace), written after the classification arm so the pass that classifies also takes the first look, and otherwise carried forward under its old stamp. A section is `destroyed` when it is not intact or its internals are at their floor; otherwise its armour plus internals against their maxima, per mille, read `green` at or above `condition_green_permille` (667), `yellow` at or above `condition_yellow_permille` (334) and `red` below: three `[tacsim.sensors]` keys, not a Rust table. The mesh hands over the sharer's OWN reading under the sharer's own stamp, adopted only when strictly newer and never putting back a section the receiver already knew destroyed; a section the gunner watches destroyed is written into the reading they hold, so `SCAN` cannot contradict the line just sent. An unidentified contact answers `X2 is not identified yet; there is nothing to read.` and an identified one with no look `No reading on BA yet: close the range for a look.`. Both readings are hashed. The frame carries a reading and its section table only for a row that has one, so an unread row publishes no chassis, and every notice object carries a `section` key, null but for `hit` and `target_section_lost` and, since 16b cut B.1, the victim's own `struck`, `struck_spent` and `section_lost` (this platform's own section, never a contact's: the B.1 entry below). A unit has at most 16 sections.
- **§5, §3.2 and §3.6 (the feeds, the ground and the three cockpits)**: as built (Q.3, Q.4 and Q.5): the per-pulse frame rides the feed named `arena.cockpit` (it was `tacsim.frame`; the schema id and version 1 did not move). A second feed, `arena.map`, is the whole arena's terrain and nothing about any unit: **surveyed ground, known to every pilot at check-out, its units not**. It is PULLED by a panel client through a query and never pushed: a seated pilot gets the ground of their own match, and a bystander, a staffer or an ex-pilot gets `seated: false` and not a word about any match. A query reply never resumes, so a panel client re-asks, at most once every 2 s, while it holds a frame of a match whose map it lacks: on every frame and, since 16b cut B.2, on a timer under the same latch (a 2 s interval on the web, the output pump's four-second tick on the TUI) and on `p` (pull) on either cockpit screen, and a map answer of `seated: false` met while a seated frame is held makes the client ask for the cockpit once, so a dead seat is ended by the cleared answer (the B.2 entry below). The cockpit feed answers the same kind of query from the world's mirror, a pilot who stops being one is sent exactly one cleared cockpit keyframe, and a pulse staged before the release is dropped. **One map-window rule, its size the only parameter**: the window is centred on the pilot's cell and clamped to the arena, north up, the pilot's `@` written first and then the contacts in frame order, the last write winning: 39 by 20 cells on the line client, each printed two columns wide with a footer in arena cells (`VIEW`, a close-in window since 16b cut B.4, which follows its panel with one line when contacts plot beyond the window and says the window is dark when it holds a frame with no ground to draw: the B.4 entry below), the pane's own size on the TUI, 128 cells with four zoom sizes on the web. **Retain-latest**: every arena payload declares `schema.retain = "latest"`, and since 16b cut B.5 the session layer keeps a marked `Keyframe` in a per-feed latest slot beside the resume ring (the ring records none of them, `evicted_seq` never moves for one, and a resume merges the slot with the ring by seq; until B.5 a ring scan dropped the feed's earlier marked keyframes instead), so two frames a second no longer evict everything else from the ring and a coalesce can never answer `TooOld`; a query reply is never recorded, so `arena.map`'s marker can never fire and the B.2 heal is what resumes a map; no proto or IPC change. The TUI and the web client each carry the cockpit on screen 0 (digit `0`, last in the strip; map, status, damage, notices, weapons and contacts; the read verbs on a key, the orders prefilled; opened once per match), on both clients the tab from a match's first seated frame until the cleared one, kept across a web rehydrate (seated-once, 16c Q21; 16b cut B.3). Parity is one canonical fixture set, `web/src/lib/fixtures/cockpit/`, read by the game crate, the line renderer, the TUI and vitest: the TUI's and the web's compositions at 39 by 20 cells, one glyph per cell, equal `view_grid.txt` byte for byte, and the line renderer's printed rows are that file with every glyph doubled (16b cut B.4), which replaces §5's golden test in each renderer. The web's decoded frame keeps no contact id, lock digits, unit number or notice text, so the observer rule is in the type.
- **§3.1, §3.4, §3.6 step 5, §4 and W6 (arena debts)**: as built (Q.6): motion no longer truncates three times a substep. The turn, the speed change and both axes of travel keep their remainders on a hashed per-unit carry, so a straight run at constant speed, a pure turn and a ramp on uniform ground end at the same place however arena time is cut, and a 3 cm/s crawl accumulates where it stood still for ever; every stop, wreck, dead engine and cold hull zeroes what was never travelled (a stop leaves the turn's carry alone). What is NOT exact: position while accelerating or while turning under way is first-order, event timestamps are substep boundaries, and the absolute substep lattice is not built, so two different partitions of one arena time still give two event lists (an owner question; the three shipped tempos share one partition). The substep bound is `(cell_cm - 1) × 1000 / max_speed`, a strict margin and not a repair: the carry is inside the one division, so the old bound was also one cell, and the stock tank stays cap-bound at 250 ms. The ledger's count of `blocked` lines still depends on the substep length (F.2's two flags: the journal on the per-substep flag's rising edge, the pilot's `Held:` on the held edge); a ruling is owed. A round that lands on an already spent section is still a `hit` line and no longer counts in the shooter's `hits`. **The share mesh is jammed by ECM, which closes W6**: a link needs line of sight at the TIGHTER of the two ends' attenuation budgets and range inside the SMALLER of the two ends' scaled reaches, each end folded through the sensor pass's own jamming and filter functions, and a field with no pod pays nothing. X.2 to X.5 have the cross-tempo tests they owed.
- **§6, §3.8 and §4 (the reflex doctrine, the fill and the dry draw)**: as built (Q.7): a reflex unit learns a stop at the journalled `blocked` event, under the ledger line's own timestamp: its route is dropped, and an occupied cell goes on a hashed avoid list (eight at most, the least recently stopped-at evicted since 16b cut E.3, aged out after 30 s of arena time, never cleared by a goal change) that the A\* plans round at the next tick. With no route the follower tests the one neighbour on its bearing by the crossing's own passability, slope and shoulder rules, and holds rather than drive at a refused edge. **`Withdraw` is not terminal.** The Withdraw stand: a withdrawing unit with a line to a classified hostile inside its reach stops, turns and fights for three decide ticks without leaving the state or losing its route (since 16b cut E.1 inside the close band and at or above `stand_min_permille` on the way home, and inside its release reach at any health once home: the E.1 entry below); a dry unit, or one under `hold_fire`, keeps driving, and an ordered `Objective::Withdraw` stands the same way unless it carries `hold_fire` (§6 "Deliberation"). The single rally: a unit that has arrived and has held no hostile for `[tacsim.arena.reflex].rally_ms` (20,000) returns to `Patrol` ONCE and then fights to the end, and a released seat rallies by the same rule. A unit that was armed at check-out and has nothing left to fire withdraws and never rallies. A shot that would overheat is held back on the engine's own line (since 16b cut E.2 the engine's own predicate: a shot that lands exactly on the ceiling is taken, as a pilot's is). **`EndReason::Spent` is a draw**: when both sides brought guns, no unit in action has a round left and no impact is pending, the match ends as a `draw` with reason `spent`, settled after elimination; running dry yourself does not lose the match. **This reverses §3.8's two-a-side minimum and the lobby ruling recorded at N.1**: with `[tacsim.match].ai_fill` on, one claimed slot in the whole centre is a solo start, each side is topped up from its lowest unclaimed pods to the larger claimed side or to `SIM AI <n>` (a side's lead or staff, `Selection` only; the number is a floor that later claims raise), and only an empty centre aborts at the boarding deadline; with the fill off both sides still need a pilot, byte for byte. The instance template and dev2 turn the fill on; the example stays off.
- **§7 (configuration values)**: as built (Q.7): every `[tacsim.arena]`, `[tacsim.match]`, `[tacsim.sensors]` and `[tacsim.harness]` VALUE is checked in the host's mapping at boot and refused by name, in the shape `[tacsim.arena.tempo].engaged_ms must be at least 50 (it is 0)`. The checks are never called from `Arena::new`, so a hand-built test rig keeps its odd numbers; they do run on a server whose `[tacsim].enabled` is off.
- **§3.3, §3.7, §3.8, §5 and §9 (extension units in the centre)**: as built (Q.8 and the cut's closeout pass, 2026-09-20): the production catalog is the FIELDED catalog, the shipped packs followed by the five extension tables (units `line_tank_laser`, `biped_mk1`, `line_tank_thermal`, `line_tank_ecm` and `line_tank_eccm` with their weapon, ammunition and sensors), while the stock catalog stays what the §9 acceptances are read against; no stock `DefinitionRef` moves and nothing new is latched. An operator pack under a shipped id and version is skipped when its digest is the shipped one and refused by file name when it is not. `SIM UNIT [<type>]` lists what the server can field, or chooses the type of the caller's OWN slot, during `Selection` only and announced to the centre (since 16b cut D.1 the bare list answers anybody in the centre in any state; the choice is still `Selection`'s); the board prints a chosen type during `Selection` only, so both sides' choices are public until the start and a seated pilot cannot read the other side's chassis off the board after it. A pod's tank may be refitted for one match: `SIM START` fits every pod of the centre to its slot's choice or else its authored type, after every refusal and before a pod locks or an id is minted, and a claimed slot that cannot be fitted refuses the start in a sentence that names the slot and no object. The route is `mush_tactics::module::refit_unit`, the second instantiate route: no eligibility test and no behaviour binding, a deployed unit refused, one governed `unit:<obj>` write, no placement, station or billet touched. The authored type is put back when the slot goes (a `SIM LEAVE`, the boarding deadline dropping the pilot) and at every return to `Idle`, the last `SIM LEAVE` included, so no pilot and no AI-filled pod inherits a choice. Stated in the help and not fixed: a legged machine pays the tracked cell cost, a bare `FIRE` on a platform with no cannon fires its first weapon, and the machines fly only a pod's authored type.
- **§3.7, §3.8, §3.1 "The seam", §3.6 and §5 (the board, the band and the arena's clock)**: as built (16b cut D.1, 2026-09-21): `SIM STATUS` takes a reader. While a match runs a player reads their own side in full and every other side as index, callsign and pilot name alone (no pod, no `seated`, no `left`, no type), a player with no slot reads both sides that way, and staff (`@arena/status`) and every other state read the whole board; the pilot's name stays because it was public in `Selection` and cannot change while a match runs. An empty weapon list on a live frame is a fact and not a gap: the cockpit falls back to the world's pre-match record only when it holds no frame at all, so a tank that lost its turret reads `This unit carries no weapon.` and `FIRE` refuses locally. `STARTUP` reads the mirror first (`Power is already online.`, `Power is still coming up; hold on.`, neither sending an intent) and otherwise stamps the one optimistic fact the mirror may hold, a `Starting` hint with four ends: a frame with power up confirms it, a frame carrying the refusal of that `STARTUP` clears it, it expires one Quiet period and a second later in world time, and an unbind drops it. A contact's range never reads `0 m`. **The pilot's band word is scope advice**: `contact_band` reads thirds of the lesser of the platform's own nominal sight (`Frame.sight_cm`, frame key `sight_m`, nought with no sensor, when the last band alone is used) and the fired weapon's last band, `beyond` past that band; on the stock tank `close` to 200 m, `effective` to 400 m and `far` beyond, a mesh plot past the sight reading `far`, and the lock's hit percentage and `WEAPONSTATUS` stay the gun's own answer. This amends the F.3 entry above: the commander's `range_band` (the deliberation brief's word and the order ceiling the reflex controller reads) still bands against the weapon alone, so a lead's `close` and a pilot's `close` were two distances until 16c `T-81`, closed at 16b cut E.1 (the E.1 entry below). Bare `SIM AI` and bare `SIM UNIT` answer anybody in the centre in any state (`Yours:` only with a slot); a non-lead who tries to set the fill is told only a side's lead or staff can (`first pilot` until 16b cut B.4). `Frames` carries the arena's `now_ms` and may be empty: it is sent when a frame exists or the arena second has moved, which on the shipped config (a 1,000 ms sensor pass and reflex decision under every tempo) is about one small droppable message per match per arena second; the board prints `arena time m:ss` from it, at worst an arena second stale, where it had read the last closed ledger chunk's timestamp. That traffic is one more world turn per arena second per live match and makes the transient `please retry in a moment` refusal likelier, which was the owner's to rule on (16c `B-13`) until 16b cut B.5 held such a line behind the commit for up to 250 ms instead (the B.5 entry below). `STATUS` prints the pilot's own scope word (`clear`, `unidentified`, `hostile`; frame key `scope`; friends count for nothing) where it printed the arena's tempo; `tempo` stays a frame key for the clients' timers and `Frame::same_as` still compares it.
- **§3.1 "Pulse phases", §3.5, §3.7 and §5 (refusals reach the pilot)**: as built (D.2): this amends the F.2 entry above twice. Any world-sent order other than a commander's `Order`, `HEADING` and `SPEED` included, that the pulse's phase 2 refuses as `Wreck`, `MotionDisabled`, `QueueFull` or `PowerNotOnline` is told to its pilot through the same `Refused` notice phase 4 stages, the order spelled by the one `IntentKind::as_str` in both phases; `Epoch` (the purge), `Channel`, `Unknown`, `Linkdead` and `NotReflex` stay silent. Every `FIRE` refusal, local or arena-side, reads `FIRE <weapon> refused. …`. `QueueFull` is told although `intents.max_pending_per_unit` bounds a queue drained once a pulse and is therefore a function of the tempo (16c `T-82`): the drop predates the notice, and nine discrete orders in one beat refuse exactly the ninth at all three cadences. The frame carries `wreck` and `immobile`, and the cockpit refuses every ordering verb on a wreck (`Your tank is destroyed and takes no orders. EXIT leaves the pod.`) and the two motion verbs on a tank that cannot move before anything is sent. A destroyed pilot and a disabled one are each told once, at the ledger event's own timestamp, what their options are (wait it out, `EXIT`, or `SIM ABORT`; notice kinds `destroyed` and `disabled`), naming nothing of the shooter, and `STATUS` marks `, DESTROYED`. **A round on an already spent section is one distinct sentence**, at most once per 5,000 ms of arena time per section, to the victim (`struck_spent`) and to a gunner who holds the target classified (`Hit on BA: turret, already destroyed.`); a gunner who holds the target unidentified or not at all reads byte for byte what he read before, every round, because any rate limit would tell him which rounds struck dead sections. A weapon that makes no heat is never refused `Overheated`, in the engine, the reflex controller and the cockpit alike (`heat_per_shot` on the frame's weapon row). `FIRE` takes an exact id, then a DERIVED short name (the initials of a multi-word component id: `machine_gun` is `mg`, shown on `WEAPONSTATUS` where no other weapon shares it), then a unique prefix, all case-folded; a word two weapons answer to names both and fires nothing. `(FRIENDLY)` marks the lock echo, the lock line and the hit on a classified teammate and gates nothing. The notice object carries `cause`, `refusal`, `order`, `weapon`, `detail`, `friendly` and `spent`; the web composes its line from them and never from `text`, and both panels print `scope` where they printed `tempo`. **The hashing rule this cut fixed**: arena state that persists past the substep it was written in and decides a notice is folded into `state_hash` (D.2's table of what was last said about a spent section, written whether or not anybody is seated; D.4's `numbered` and `receiver`); a table filled and drained inside one substep, a function of state already folded, is not.
- **§3.7, §3.8, §3.1 "The seam", §4, §6 and §7 (the lifecycle's ends)**: as built (D.3): every end before `Running` goes through one funnel (release as `Aborted`, put the stock tanks back, one announcement): `SIM ABORT` from `Selection` or `Boarding`, the boarding deadline's and the boarding settle's empty arms, and every refusal of the `Boarding → Running` transition, which now calls the lobby off (`Match n could not start: …. The pods are free again.`) and still returns its error; so a `SIM UNIT` refit never outlives the lobby that chose it, and there is one writer of `Idle`. A non-staff `SIM ABORT` in `Selection` is refused, a lone claimant's included. Two new `Stop` reasons, world-driven like `SIM ABORT`: **`SIM CONCEDE`** (a side's lead, `Running` only, no staff case) ends the match `Conceded { winner }`, a recorded win for the other side, and **the unattended end**: `[tacsim.match].unattended_ms` (300,000; 0 is off, otherwise at least 10,000, refused by name) of world time after every slot of a slotted `Running` match has gone `left`, the lobby's tick stops the match, reason `unattended`, outcome `timeout`. Both journal `match_ended` and commit cells. The stamp is cleared, and the end announced, only by a stop the arena accepted, so a momentarily full inbox is retried on the next tick and never disarms the path; a staff `@arena/start` match has no slots and is never stamped, and a seated pilot whose tank is destroyed still counts as attending. This amends the N.1 entry above: a released tank that was still cold orders its own startup at its first reflex tick, comes online `match.startup_ms` later and drives home still in `Withdraw`. A refused check-in releases the seat and retries once as void, never skipping the unit; `@arena/release <unit>` (staff) clears a stranded `deployed` marker and frees its seat, and is refused for a unit a live match holds. A cockpit verb typed from a pod answers that the match has not started yet only for a billet on a roster tank while the centre is `Boarding`. `SPEED` names and enforces both ceilings, bare `LOCK` is usage, `EXIT` says one line to the actor, the board drops a dash that only repeats the state, and `STATUS` brackets a refitted chassis.
- **§3.6, §3.7, §4 and §5 (what a contact is called, and the receiver)**: as built (D.4): a loss cancelled inside the hysteresis window is silent unless the live row is still `unknown` and its name moved; then one line says the regression (`BA is unidentified again: X2.`, notice kind `unidentified`), the old name is never an alias, and `LOCK BA` is still the one byte-identical refusal. A contact row records whether this platform's own sensor pass numbered it (hashed), so a mesh arrival is one `New contact BA` line and only a hold the platform opened itself says the rename; the acquisition notice carries the live class. **A platform knows its own receiver is degraded**: the sensor pass keeps the jamming fold it already computed (`jamming_at` still runs once a pass and no perception counter moves) as `UnitState.receiver`, hashed, `None` for no fold, for a fold a filter wholly recovered and for a platform whose pass no longer runs; the ledger journals `receiver` on the transition only, the pilot reads `Receiver degraded: something is jamming the sight.` and `Receiver clear.`, `STATUS` prints one line (`receiver degraded: 15 of 40 through cover, 60% reach`) and the frame a `receiver` key. No source, no bearing, no team and no id; NOT no count: the integers read deeper under stacked emitters and let a pilot walk a bubble's edge, which is declared as a limit and put to the owner with the tightening named (a thresholded word, the integers off the wire). `section_destroyed` and `unit_destroyed` carry `by`, and the gunner's lines read it, so a section lost to a consequence after the shot resolved stages no gunner line. This amends the Q.1 entry above: a locked contact's range step is the plot's age, as the contact's own row carries it on the same frame, times 17 m/s, rounded up to ten and held between 10 m and 50 m (10 m under 648 ms, 50 m from about 2.4 s), so a one-pass plot reads to 20 m and the step publishes nothing the pilot does not hold. Left to cut B and built at 16b cut B.1: a `receiver` row on the panels, and the old name on the web's regression line (16c `T-86`; the B.1 entry below). Open: a mesh share that cancels a held loss renames `X1` to `BA` with no line (16c `B-74`), and the hysteresis expiry is swept once a pulse, so past the deadline the two tempos tell two stories (16c `T-83`).
- **§3.7 "Pods", §4 "Content packs" and the way in**: as built (D.5): the alias `l` came off five shipped exits, and pack apply refuses, before its first write, an exit any of whose name segments equals the name or alias of a command the kernel or the host registered, the words that work in every room (`exit '…' uses 'l' as a name; that is the command 'look', …`). A module's verb is let through on cut F.4's ruling: both shipped bus exits are named `board` on purpose and the cockpit gate stands aside outside the centre for it, and a contract test over the production registry counts that overlap (two rows, both `board`) so the next one is ruled on by a person. The check runs on created and updated entries, so a standing operator pack is not refused until the entry changes; `@dig`, `@open` and `@name` are not held to the rule (16c `B-75`). `look <keyword>` tries `detail:` on the room and the looker as before and then each prefix a module registered inside an attribute namespace it owns, the keyword folded (trimmed, lowercased, spaces to `_`); the atlas registers `atlas:detail:`, so the Welcome Hall's sign reads. Details authored on ITEMS (the selector's screen, the pods' panels) are still unreadable, which keeps 16c `B-12` open for that half. `neighborhood.ooc_area` is at version 6 and `neighborhood.staff_lobby` at version 4 (requiring OOC 6); a standing world takes them by hand, OOC first, and its exits are remade. Three help pages of the deleted rulesets are gone and a contract test holds the head word and switches of every code span in `docs/help` to the production command table.
- **§3.1 "Tempo", §3.6 and §6 (the machines fight on the shipped ranges)**: as built (16b cut E.1, 2026-09-21): a platform's fighting reach is the lesser of its own nominal sight and the band of the weapon it would fire (`UnitState::sight_cm`, `loaded_weapon`, the default weapon while it has rounds else the first with rounds, `fight_limit_cm`, `fight_inner_cm`), which is cut D.1's ruling for the pilot's band word applied to the machine, because pricing off the cannon's 1,500 m inner band is past every plot a platform can hold on this map (16c's Q9, departed from). The reflex controller builds three distances a tick with the doctrine band applied AFTER a commander's order cap: `engage_cm` is `engage_band_permille` of the capped inner reach, `release_cm` is all of it and is also the ceiling it fires to, so the hysteresis is three tenths under every order (stock tank 420 and 600 m on doctrine or `far`, 280 and 400 under `effective`, 140 and 200 under `close`), and `stand_cm` is the `close` band (200 m). `Approach` becomes `Engage` inside `engage_cm` with a line and locks and fires on that same tick, still rolling; inside `engage_cm` with no line it drives to the plot's own cell; outside it drives to a stand-off cell whose distance is floored first and bounded by the range last, so the goal is never behind the machine (16c `B-05`). `Engage` gives up past `release_cm` and closes again (`T-47`); a tank with a dry cannon closes to its coaxial's band (`T-46`). The withdraw stand has two rules: on the way home only inside `stand_cm` and at or above `[tacsim.arena.reflex].stand_min_permille` (175, refused at boot above `withdraw_permille` in a sentence naming both keys), and once home, or with no home to go to, inside `release_cm` at any health, because a machine with nowhere further to run that never replies is the parked target cut Q.7 removed (`B-39`; the departure from 16c's Q8). The derived `tempo.alert_range_cm` is capped at the window's diagonal (362,000 on the proving ground) and `Engaged` is triggered by a classified hostile inside the inner fighting reach, so a mesh plot between 600 and 900 m is `Alert` (`T-32`). `contact_band` is the one band word for pilot and commander and `RangeBand::ceiling_cm` is its inverse, taking the sight (`T-81`). `match_::start`'s custody literal is `arena_def::custody_unit`, and a shipped-scale rig (`crates/tacsim/tests/support/mod.rs`) builds what production builds. The help says the doctrine in one paragraph. Not built: firing on the move while closing (a machine closing from 600 m to 420 m does not shoot). Amended by AA.1 (below): a machine that cannot close enters `Engage` at `release_cm`, counts as home wherever it stands, and ends an `Advance` it holds.
- **§3.3 "A wreck", §6 "Deliberation" and "Provider failure" (orders, rally and knowledge)**: as built (E.2, 2026-09-22): a dead target is knowledge (16c's Q11): a contact carries `known_dead`, written once on the shooter's own hold beside its pilot's `Kill` line and never carried by a share, and a held reading whose destroyed sections include one the target's profile makes fatal reads dead too; one predicate, `Arena::contact_reads_dead`, replaced the five live reads of the target's wreck flag in the controller and both tempo triggers, so a machine drops a target at its own kill or a fatal reading and everyone else holds the track until it expires (about 3 s) and may spend a burst on it, while lines of fire, locks and impacts stay physics. An `Advance` whose cell no route reaches ends the order for that unit at the tick its search failed, through the existing `order_expired` line and the squad's next `OrderExpired` signal, only the ordered cell's own failed search doing it (`B-41`; the commander is not yet told why). No rally while an `Objective::Withdraw` order stands (`B-40`). `would_overheat` is the engine's own predicate (`T-55`). An order that omits `ttl_ms` lives `deliberate_min_ms`, carried as a field on the signal and the brief that the prompt does not print, and its default horizon follows it (`T-56`).
- **§3.4 step 4 and §5 (driving)**: as built (E.3): an occupancy refusal is a hold before it is a stop. The unit is clamped at the boundary with its travel carries zeroed and its speed kept, journals nothing and is told nothing, until a hold on one cell has outlasted `[tacsim.arena].occupied_stop_ms` (2,000, arena time; 0 is the old rule) and becomes the literal stop with its `blocked` event, its `Held:` line and the machine's remembered cell; an accepted crossing, a refusal of another cell or cause, a substep that moved the unit and a zero desired speed clear the hold (`UnitState::occupied_hold`, hashed). Ground, edge and slope refusals stay literal (16c's Q10); 16c's sketch of no event at all was corrected because the remembered stops learn only from the event, and a machine held by a wreck in the ford would have pushed at it for ever. During the hold a pilot's frame reads the kept speed and no `HELD`, and `Held: something is in the way.` arrives after the hold, never sooner than before. Two seconds is clean for a column of up to eight (units integrate in id order); a longer compressed column still matures its tail and teaches a teammate's transient cell, measured and not cured. The controller orders the speed it can still stop from, `isqrt(2 × brake × distance)` with one tick of lookahead, so machines brake for their goals and the duel's first shot is fired braking, the shooter stopped within seven seconds (`B-42`); the remembered stops evict the least recent stamp (`B-43`); a routeless machine's next cell on its actual heading is checked every substep before motion and held through `apply_intent` (`B-44`, fixture-proved until `T-18`'s bluff). Under a ragged pulse the promise is the outcome and a hold's maturity within one substep, not one event list.
- **§6 `Patrol` and §3.8 (the endgame)**: as built (E.4): the three shipped-scale cases (a hurt machine against two, a healthy one against one, a dry loser; the production roster on the authored ground, no pilots, shipped doctrine, hurt by outcome to 227 per mille) were run first un-fixed and showed 16c `B-04`'s cause: the dry loser parked on its entry cell in `Withdraw` while the winner walked the far half of the map for fifteen arena minutes. Two fixes: the search, a machine that loses its contact drives to its own last plot for `[tacsim.arena.reflex].search_ms` (60,000; 0 is off, otherwise `decide_ms` to 600,000); and the hunt, a patrol that has held nothing hostile for `hunt_after_ms` (90,000; 0 is off, otherwise at least `decide_ms`) crosses to another side's entry zone by `far_entry_cell`, which never answers an impassable cell (`entry_cell` untouched, a side setting out together and not staggered). `Patrol`'s goal is, in order, a movement order, the search, the hunt, the waypoint walk; arrival, age, a hostile held, `Withdraw`, an order and the `unreachable` edge clear them (`last_plot`, `unheld_since_ms` and `hunt`, hashed). All three cases end by elimination inside 900,000 ms (195,750 / 188,750 / 286,750). The `no line` edge, the `avoid` clause and `Spent` at three cadences have the tests `T-42` named. The help says a machine that loses you goes to where it last saw you, and one that has seen nobody for a minute and a half crosses to look. Known and left (16c candidates): the shooter searches the wreck it knows it killed, and a lone hunter can orbit its next cell in water at a one-cell turn radius.
- **Cut L documentation correction (2026-09-22)**: A1's split twin lives in
  `mush-gateway/tests/arena_story_split.rs`; the conceptual seam includes `Deliberate`;
  cockpit panels currently send keyframes only; N.2 deliberation uses bounded
  `LlmService` requests rather than durable K7 decisions; N.1/Q.7 already provide
  AI takeover. CI exercises the cost report with `TACSIM_LLM_COST_MOCK=1`, without
  a provider key. The live cost run remains an explicit owner-funded opt-in.
- **§6 (the immobile machine)**: as built (16b cut AA.1, 2026-09-22): one reading, `UnitState::cannot_close` (`limits.motion_disabled`, a destroyed engine or both tracks, or `mobility.max_speed_cms == 0`, an authored hull with no speed, which still turns in place). `Approach` chooses its closing edge off it: `release_cm` for a machine that cannot close, `engage_cm` otherwise, so an immobile armed machine enters the existing `Engage` arm anywhere inside the reach it may fire across (the ceiling `engage_orders` holds and the edge `Engage` gives up at, so no oscillation; `hold_fire`, an order's band cap, power, ammunition and heat all hold because the arm is unchanged), where before this cut a tank whose engine was killed between 420 and 600 m re-planned a stand-off cell it could not drive to every tick, never fired again and, being armed, held its side up for the rest of the match (16c `B-76`). `Withdraw`'s `arrived` reads true for it, by the same argument as a machine with no home, so a hurt immobile tank stands and shoots back inside `release_cm` at any health and rallies once (Q8 extended one step past E.1's departure; drop the `cannot_close()` disjunct to reverse). An `Advance` it holds ends for it on its next decide tick through `B-41`'s own `plan.expired` and `order_expired` line, so the order layer cannot reopen the window for a ten-minute TTL; `Hold` is untouched. `best_hostile` has no reach term, so an immobile machine whose utility pick is a held plot past its `release_cm` still never fires (a residual, with the shared-sightings one); a mobile machine boxed in by wrecks or terrain keeps the same window, because the reading is a hull fact and not the map. The help's machines paragraph says it in one sentence (600 m for the stock tank). Hash sequences move wherever an immobile armed machine sits inside its reach or holds an `Advance`; no pinned value existed.
- **§3.8 (the dropped lead)**: as built (AA.2): one writer of the side-lead succession, `Centre::resettle_leads`, rebuilt over both teams from the slots as they stand (the recorded lead stays while that account holds a slot on the side and, since 16b cut B.4, has not left it, else the side's first remaining claimant in claim order who has not left, else no entry), called by `SIM LEAVE` in place of its inline block and by the boarding deadline after its drop inside the same borrow, so a side whose lead never boarded is led by its first seated pilot and can `SIM CONCEDE` with a result (16c `B-78`). No announcement; the concede gate, `SIM START`'s and `SIM AI`'s gates, the claim rule and every refusal are unchanged, and the help's lead line says who leads after a drop. Built at 16b cut B.4: `slot_left` resettles the leads as soon as it stamps a slot `left`, so a lead who disconnects or `EXIT`s mid-match hands the side on at once and a teammate can concede with a result; every refusal, help line and doc comment says lead where it said "first pilot" (the B.4 entry below).
- **§3.1 "Failure" and "Custody" (the stalled terminal pulse)**: as built (AA.3): the arena task reads `took > tempo.stall_ms` before it honours a pulse's own end; a stalled pulse that also ended the match is voided at the task (`MatchEnd { Void, t_ms: arena.now_ms }`, no `arena.stop`, no second step: `Arena::stop` is first-wins and `Arena::step` echoes an ended arena's original end before it reads the stop), so `Task::end` sends no units and the world restores every check-out snapshot; an end already `Void` or `LedgerOverflow` keeps its word, the non-terminal watchdog is unchanged, and the stalled pulse's chunks are still published as evidence (16c `B-79`). Recorded: the forced void is NOT journalled: the black box ends in whatever end the arena journalled (`match_ended`, `aborted`, or none for a ledger-full timeout) and carries no `void` event while the result and the seam say `void`; the journalled variant needs the library's ended short-circuit and first-wins stop to admit a later `Void`, only toward void being the safe direction. Built on 16c section 5's Q41 default (yes, void and every unit restored, not journalled) rather than the owner confirmation `B-79` was gated on; reversed by reverting the terminal arm of `Task::pulse` alone. Not built: a mushd-level stalled-terminal custody lens (a deterministic terminal stall through the server needs a racing fault verb or a `max_duration_ms` the validator refuses; the injected-panic lens already proves the world's void path).
- **§5, §3.7 and §3.6 (the web pilot stays on screen 0, and what the panels print)**: as built (16b cut B.1, 2026-09-22): on the web the composer and the command palette are the shell's, mounted under every screen but the channel screen (which keeps its own box), so an order from a contact click, a cockpit key or the `Enter type` chip lands in the box on the screen the pilot is looking at and `focusComposer` no longer switches screens (from the channel screen alone it goes to the scene screen); the once-per-match switch to the cockpit is taken on the frame that owes it, the draft, mode, history and focus surviving, and a prefill that is not a speech trigger drops a sticky Say or Pose so it is sent as a command (16c Q5). While seated, each cockpit notices ring (the web's since B.1, the TUI's since B.3) also carries the pilot's own `Sys` replies, because on screen 0 that ring is the only reply surface (16c Q20, extended); the bridge's per-pulse copy of a frame notice carries the span tag `arena-notice`, so the ring composes that notice from the frame alone and never rings it twice, and a `Reply`-wrapped `Sys` (a query answer) never rings. The web's default window stays 128 cells (64 would hide the 330 to 600 m band the sight covers), and a hostile or the lock is labelled at every zoom, with an 8 px font and a one-pixel halo under 6 px, overlapping forced labels skipped in frame order and the lock's never; `+` zooms in and `−` out; a late `arena.map` of another match is dropped while a frame is held; an unknown `SCAN` band letter is uncoloured. On the wire `status_frame` gains `was`, the old name on a `classified` or `unidentified` line and null otherwise (`FRAME_VERSION` still 1, no proto change), and both status blocks print `receiver 15/40  60%` after the heat row and `DESTROYED` or `IMMOBILE` on the speed line; `web/src/lib/fixtures/cockpit/notice_lines.json` pins the wording of every notice shape for both clients (`X2 identified: BA`, `BA unidentified again: X2`, `held` alone on a null cause, `hit on a dead section: left_track`).
- **§5 and §3.2 (the ground heals, and the web screen is tested)**: as built (B.2): a lost `arena.map` heals without waiting for a frame: the TUI's output pump asks on its four-second WHO tick and the web on a 2 s interval, both through the one 2 s latch the frame path arms, so the two paths never ask twice inside it; `p` (pull) on both cockpit screens re-asks the cockpit always and the map only when the ground is not held (`g` is the pane-jump key both clients reserve); a map answer of `seated: false` while a seated frame is held asks for the cockpit once, so a seat that died with a restarted world is ended by the cleared answer instead of feeding the heal for the session, and the TUI's hello arm asks the cockpit off screen 0 for the same reason. The web notices ring is emptied on a change of match before the new frame's notices land, and a full rehydrate clears the store through `clearForRehydrate`, which keeps the spent screen switch and the hello pass's map ask, so a connection blip neither drags the pilot back onto the cockpit nor pulls the terrain twice; the cleared cockpit frame is the last in the ring only until the pilot is seated again. `CockpitScreen` is mounted under jsdom (`svelteTesting()`, one opted-in file) against the committed fixtures with a recording 2D context that pins the marker colours, the wedge, the lock ring, the legend and the labels at 128 (5 px, the lock alone) and 64 cells (8 px, all); the terrain is painted once per window into an offscreen canvas keyed on the window and the map's identity and copied with one `drawImage` under the markers, so a second identical frame costs no `fillRect`. `MAP_HEAL_LATCH_S`'s rationale quotes the tempo config's 5 s / 1 s / 0.5 s.
- **§5 and §3.7 (the TUI cockpit at eighty columns)**: as built (B.3): the TUI cockpit's map pane wants 36 columns and every side pane 22, so at 80 columns no pane is dropped for width and the contacts pane (the one `Enter` prefills a `lock` from) stays; every panel row on every screen wraps at its pane's edge with the highlight on every wrapped line and the scroll anchored on the selected row's first line, the map rows never; the status pane's `min_h` is 10 (callsign, power, a speed line that wraps with `DESTROYED` or `IMMOBILE`, a heat row that wraps with `OVERHEATED`, the receiver row, scope and a lock that wraps past a kilometre) and `draw_body` gives every pane of a column at least its minimum, so the Lock row is painted at 80x24, 100x30 and 120x34 and `damage` goes for height below about 31 rows (at 120x34 it shows three of its five sections without scrolling, an open owner call). The map legend names the contacts that own their window cell in frame order, the `@` first, on the TUI and the web alike, cut to one line with `N more` when the pane is narrower, and the map pane sheds its heading at three rows and its legend at two. The notices pane is a `Panel` over a `PanelStore` ring of 50 lines composed from each frame's notice fields by `notice_words` (the web's `noticeLine` rule for rule, pinned against `notice_lines.json`, every field cleaned and bounded, never the server's text), plus the pilot's own bare `Sys` replies while seated (split at line breaks, at most 200 characters a reply), reset per match and emptied by the cleared frame, `Nothing yet.` while seated and empty and `Not in a cockpit.` otherwise. The cockpit tab stands from a match's first seated frame to the cleared one on both clients (the TUI clears `cockpit_seat` and moves a pilot off screen 0; the web gates the screen on `cockpitSeated`, kept across a rehydrate), `v` is gone from the TUI cockpit, `PanelStore::forget` drops a feed from all three of its maps, and the output pump records every `Room`, bare or `Reply`-wrapped, into the TUI manifest before its line/TUI branch, so `@tui` opens on the room a walk in either mode reached, with its exit keys seeded. One split-gateway test opens `@tui` on screen 0 through the production split and reads the pulled ground across the IPC boundary.
- **§5, §3.7, §3.8 and §3.6 (line mode, the attach look, the range that holds, and the lead who leaves)**: as built (B.4): a cockpit frame that renders to nothing on a line client no longer erases and redraws the pilot's half-typed line (the SSH pump renders into a scratch buffer first and writes nothing when it is empty). `VIEW` is a close-in window of 39 by 20 cells around the platform, printed two columns a cell by the line renderer through an additive `cell_columns` key (`origin_cell` and `extent` beside it) with a footer in arena cells (`x a..b  y c..d  of WxH  you at (x, y)  (N m a cell, north up)`), and says `The window is dark: there is no ground under the panel to draw.` when it holds a frame it cannot draw; the 30-cells-north symptom of 16c `B-24` is `CONTACTS`' and the beyond-window line's to cover, not the window's. A wrapped `SCAN` run counts the comma it takes and never passes 76 columns. `BOARD`, `EXIT` and both ejections show the pilot the room they land in, after the sentence that explains the move (`Ctx::show_room` at the callers, never inside `put_out`). The attach look is a core `room` query (`RoomQuery`, registered beside `who`, answering the actor's own location through the live lens as `Reply { id: 0, Room }` to the one connection that attached, `You are nowhere.` otherwise), sent by `enter_game` in place of a typed `look`, so a second terminal or tab no longer prints the room on the first; the web routes a replied `room` like a bare one and the Discord follow's DM unwraps it; both halves deploy together, since an old server answers `Unknown query`. Each unlocked contact row's range is held on the bridge's mirror against the figure the pilot was last sent (`held_range_m`: within five eighths of the step of the held figure it stands), settled from the previous mirror of the same match on every push, and `RANGE`, `CONTACTS`, `VIEW` and both panels print the held figure; the lock keeps its age-derived step and no memory. A lead who leaves the tank mid-match hands the side on at once (`slot_left` calls `resettle_leads`, which keeps or promotes only a slot that has not left and gives an all-left side no entry), and every refusal (`Only a side's lead or staff can start the match.`, `Only a side's lead or staff can set the AI fill.`, `Only a side's lead can concede for it.`), help line and doc comment says lead.
- **§3.1 "The seam" and §5 (a player who types ahead is served)**: as built (B.5; doc 09's admission contract amended in the same commit): a typed line, a panel query or a reply-carrying world read (`NameFree`, `UnlinkPlayer`, `InInputContext`, `ConnectionReady`, `LegalActions`) that meets another action's commit is no longer refused at once. The world task holds it in a bounded defer queue (four typed lines a connection, 64 entries in all, 250 ms from arrival on the runtime's clock) while it keeps polling its inbound, drains the queue one entry a turn once the receipt lands, before the lifecycle slot and before anything new, and refuses a typed line still waiting at the bound with `The world was busy too long for that line; please retry in a moment.` (`RefusalReason::CommitTimedOut`, its tail the phrase every test rig's retry loop matches), while a timed-out query or read keeps its typed `Busy`; the lifecycle messages (`ConnUp`, `ConnDown`, `SessionGone`, `Apply`) keep the one deferred slot, `Composing` is still dropped, and a mutation is never queued. A barrier that arrives behind queued entries waits behind them, so `barrier_result`'s contract (everything sent before the call has been processed) holds; one that meets a full queue is refused `Busy`, retryable and retried by the gateway, rather than answered ahead of the receipt. `commands_total` counts a line once, at admission or final refusal, and `mush_command_deferred_total` and `mush_command_defer_timed_out_total` show the queue at work (16c Q7; the measurement was cut AA's two sweep flakes, which are its regression fixtures). This retires the D.1 entry's open ruling on `B-13`. In the session layer a retain-latest `Keyframe` lives in a per-feed slot beside the resume ring (the Q.3 to Q.5 entry above, amended in place).
- **Before IC play** (additions, not changes): per-pulse state durability on an arena writer lane with
  disjoint key ownership; resume from snapshot plus ledger tail; the 10 km arena.
  AI takeover of dropped human slots is already built (N.1 and Q.7).

---

## 12. Decisions (owner, 2026-09-16, binding)

| # | Decision | Source |
|---|---|---|
| D-16-1 | One arena task per map; the arena is the single writer for its facts while a match runs; the seam is the eight messages of §3.1; never a shared lock | 16a Q1 B, review |
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
| D-16-16 | Reflex controller in the game crate; bounded deliberation through `LlmService` per squad (N.2 amendment); provider failure is invisible to the pulse | 16a Q37 to Q39 |
| D-16-17 | W1 to W16 are ratified world facts | 16a §9 |
| D-16-18 | Extension order: lasers, bipeds, thermal, ECM, ECCM, elevation | 16a Q40 |
| D-16-19 | The harness follows D-H-8 in CI (report-only, on demand and scheduled) and asserts under `TACSIM_ASSERT=1` locally or on a dedicated runner; an owner wish to gate CI on it supersedes D-H-8 and is recorded here if given | review |
| D-16-20 | The shipped `mush-sim` combat verb lane is retired; the arena is the only combat lane | review |
