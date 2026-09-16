# 16a — Combat rebuild: owner survey

Draft 0.1, 2026-09-16, written against `main` at `5da8d6a`. This is the survey that precedes doc 16, the
combat requirements document. It follows the shape of 14a: the givens, what the tree actually has, the
questions with recommended options, the facts still needed, the defaults taken unless vetoed, and the
outline of the document to be written once the answers are in.

Answer inline. A question with no answer takes its recommended option. Where an answer changes a
standing ruling, the ruling it changes is named so the amendment can be recorded in the older doc.

Inputs: the owner's directional decisions of 2026-09-16 (§1.1), the standing rulings in docs 06, 07, 09,
10 and 14 (§1.2), the review of the `codex/simulator-vertical-slice` branch (§1.4), and three fact-finding
passes over the tree (§2).

---

## 1. The givens

### 1.1 Owner direction, 2026-09-16

1. **One game.** The codebase is oriented around the 2350-era TacSim setting (docs 10, 11, 15). Loadable
   rulesets are retired. Shadowrun contributed the mission loop; World of Darkness contributed scale of
   play, intrigue, per-observer perception and character-to-character play. Those are platform
   capabilities and stay. The character rulesets themselves do not.
2. **Platform provides concepts, game config provides specifics.** The ten commodity families, units,
   weapons, sensors, prices and similar are data in config files, never Rust enums. What a piece of data
   *can do* is a closed vocabulary of behaviours in Rust that the data picks from (M32's "closed verbs,
   open nouns", generalised).
3. **Option C for the simulator.** The Codex branch is a research spike. The combat slice is rebuilt thin
   on `main`, reusing designs and small pure modules from the branch, never its ownership or scheduling
   machinery.
4. **The end-to-end story that doc 16 must deliver,** written as the basis everything else stacks on:
   create a player, walk to the simulation arena off the OOC Area, choose red or blue, a second player
   chooses the other side, both enter their assigned pods with default tanks loaded, navigate a map with
   terrain but no elevation, identify each other through visual sensors, fire tank cannons until one
   blows up. Then, in order and without rewrites: lasers alongside projectiles, bipeds and their hit
   locations, another sensor, ECM, ECCM, elevation.
5. **Scale, early.** Hundreds of units moving on a regional map (NPC patrols at key spots), a dozen a
   side actively fighting, two to ten weapons per unit firing every one to fifteen seconds, four to ten
   sections per unit with armour and internal counters, a substantial share of units AI-piloted, and no
   external client (inputs are trusted; all logic is server-side). A scale test comes before feature
   breadth.
6. **Lore serves the engine.** Because the game owns its universe, world facts are chosen so the
   engine's hard problems are bounded in-world (§9).

### 1.2 Standing rulings that bind this survey

| Ruling | Where | What it fixes here |
|---|---|---|
| Continuous pulse combat with persistent orders; ground pulses of about one to two seconds; fire, launch, dock and similar are bounded discrete actions | doc 10 §12 **2A** | The pulse is seconds, not 100 ms. The Codex branch's 100 ms budget contradicted this ruling |
| Deployed assets can be disabled, captured, salvaged or destroyed; most defeated units become disabled before unrecoverable | doc 10 §12 **3A** | "Blows up" is the end of a chain that starts at "disabled" |
| Ten commodity families; ammunition economic at magazine or lot scale while weapons track shots internally | doc 10 §12 **4A**, T4, T5 | The resource list is fixed at ten families; per-shot counts are unit truth |
| Continuous controls are latest-value-wins; discrete actions are sequenced and bounded; no unbounded catch-up; integer or quantised arithmetic for authoritative state; every commit records tick, intents, RNG position and a state hash | doc 06 §6.4, §6.5, §6.6 | The intent and determinism contracts are settled |
| Due domain ticks enter the single-writer world loop as scheduled work; no task or channel per unit or domain | doc 06 **§6.3** | Revisited in Q1 because of §1.1 item 5 |
| The `DomainDriver` shape: pure `step` over an immutable snapshot returning a proposed commit; no I/O, no tasks | doc 06 §6.7 | Kept; the trait on `main` lacks `step` entirely (§2.3) |
| Hybrid tactical AI: deterministic layer owns pathing, legality, reactions and link-loss; an optional LLM deliberator returns schema-validated plans inside a bounded application window with decision tokens and stale rejection | doc 07 §4, §5 | The two-layer AI is already designed; this survey scopes it |
| Link-loss fallback policy and reclaim | doc 07 §1, D1 | Disconnect handling is not re-decided here |
| Versioned definition registry, one action-plan-commit protocol, recorded randomness, generic resources and effects, authority and leases, truth versus observer projection, lifecycle and recovery; gates 2 (sectional damage), 3 (hidden-contact targeting), 4 (stale NPC action), 5 (restart matrix) | doc 09 C1, C2, C5, C7, C8, C9, C11 | The kernel contracts the rebuild rides on |
| Engine-defined surface, cover, opacity, locomotion vocabularies; engine-answered `line_of_sight`, `direct_fire_path`, `beam_path`, `move_cost`, `path`; the TacSim ground driver at 1000 cm cells; square cells only; layered 2.5D elevation | doc 14 §5, §3.5, D-14-4, D-14-6 | The spatial substrate is decided |
| The engine defines the world; rulesets register definitions | README superseded item 5 | Becomes "game config registers definitions" (Q29) |

### 1.3 Rulings this survey proposes to amend

- **doc 06 §6.3** (no per-domain task): amended by the owner's Q1 answer to "one writer per authority;
  a running arena is an authority for its own facts", with the custody protocol as the seam.
- **doc 14 §5.3** and README item 5 ("rulesets register"): Q29 replaces the ruleset with the game
  config and the C1 registry.
- **doc 10 §8** (combined arms with drones, VTOLs and aerospace on the tactical map): Q34 scopes tactical
  air out of the first builds through a world fact, without touching strategic aerospace.
- **doc 04 §3** (the `mush-ruleset` plugin API, one ruleset per server): Q30 to Q33 retire the
  pluggability and keep the sheet library.

### 1.4 What the Codex branch contributes

The review of 2026-09-16 found a mostly sound engine that was never composed into the server, a lethal hit
that could not be published, ticks of 238 to 380 ms against a self-set 100 ms budget, about 57k lines of
duplication, and a red CI. Under Option C the branch is a reference, not a source: its stock catalog and
three-room centre content, its pure mechanics files (ground drive, turret drive, weapon cycle, flight)
and its "AI submits ordinary commands" pattern are worth reading; its authority tree, version ladders,
exact rational geometry, sensor stack and migration lanes are not brought over.

---

## 2. What the tree has

Verified on `main` at `5da8d6a`. Paths are under `crates/`.

### 2.1 Substrate

- **One writer.** A single tokio task owns `Engine` and `World` (`mush-server/src/world_task.rs`); no
  lock on `World` exists anywhere. Per iteration: step and fire due work, resolve audiences, snapshot
  `World::take_dirty()` into one `WriteBatch`, await the store receipt, release output. Every engine entry
  is wrapped in a `catch_unwind` firewall; panic means an orderly non-zero exit, never abort
  (`mush-contract-tests/tests/panic_budget.rs`).
- **Off-loop work has a seam.** `Effect::ModuleAsync` snapshots context on the loop, spawns the work
  (worldgen, NPC turns, atlas window composites via `spawn_blocking`, weather, ST adjudication), and
  re-enters through `WorldCommand::ModuleAsyncDone` into `GameModule::on_complete`
  (`world_task.rs:9100-9330`). The durable outbox worker (`durable_outbox.rs:437`) is an independent task
  whose state lives in the store, delivering typed `BoundaryFact`s at least once.
- **Store.** Redis 8 through `fred`, write-through, one `MULTI/EXEC` per batch, three writer lanes merged
  in one acceptance order (`persist.rs:534-580`). Bundle ceiling 16 MiB, outbox payload and action-state
  record ceiling 256 KiB. `mush_core::Ledger` is a payload-agnostic chunked append-only log with five
  consumers (canon, scenes, economy, org, privileges). There is no WAL, no event store, no
  event-sourced rebuild; world objects are whole-object upserts.
- **Process.** `mush-gateway` supervises `mush-server` over IPC with hot restart; `mushd` is the
  all-in-one for local testing. No task registry, no `JoinSet`, no cancellation token; long-lived tasks
  are spawned in `spawn_persist_with_gateway_mode` and joined in both shutdown tails.
- **Output.** `OutputEvent` (`mush-proto/src/event.rs`) carries no audience; routing is
  `Audience`/`DeliveryClass` (`mush-core/src/output.rs`). Three renderers exist (ANSI line, TUI, web).
  Per-observer projection is one hook, `can_perceive`, at the single fan-out site. No output-side rate
  limiting or digesting exists.

### 2.2 Time

- `DomainClock` is `Passive` or `Pulse{period_ms,..}` with a floor of 1,000 ms and both `period_secs` and
  `tick_at` flooring to whole IC seconds (`mush-sim/src/data.rs:217-342`). The pulse is a `Coarse(1s)`
  module tick whose deterministic half is a prepared authoritative commit (`mush-sim/src/tick.rs`).
- `TickCadence::Fast` exists; the only sub-second registration is the `tacseam` stub, not composed in
  production. `DurableSchedule` is the durable timer (`mush-core/src/durable_schedule.rs`).
- Nothing runs sub-second in production today.

### 2.3 Domains, placement, control

- `DomainDriver` has four methods: `name`, `definition`, `create`, `closed` (`mush-sim/src/driver.rs:28`).
  **There is no per-tick driver hook.** Domains promote and demote (`MAX_ACTIVE_DOMAINS = 8`); over-cap
  requests queue.
- `Pose` is f64 region metres plus `facing_deg: u16` plus opaque posture (`data.rs:452`); windows are
  integer centimetres with `cell_cm` (`data.rs:547`). Placement is one `Transition` primitive with strict
  plan/apply; membership is derived by scanning placements.
- F3 control: `ControlLeaseKey{subject, channel}`, epoch and pending handoff (`mush-core/src/control.rs`);
  `seat::request_seat_activation` is the one mechanical seat seam; channel grants are a pure projection
  of station assignments (`mush-sim/src/links.rs`); one focus per account (`focus.rs`).
- Registered today: `@unit/{create,show,status,damage,remove,recovery}` (staff, M33) and
  `+heading +thrust +posture +fire +launch +dc +dock +eject`. Every combat verb is a selector that stages
  one `tactical_intent` and writes nothing (`mush-sim/src/combat.rs:7-14`).

### 2.4 Units, terrain, movement, combat, perception

- **Unit model (M32).** `TacticalUnit` with sections, facets, `ScalarCell{min,max,current}`, components,
  opaque capabilities, `MobilityProfile`; seven mutation verbs, flat plans, refuse-not-clamp, a journal
  that is not the replay substrate (`mush-tactics/src/unit.rs`). Sections have no topology. Persisted as
  a monolithic `unit:<obj>` document. **The production catalog is empty**; every `UnitSpec` in the tree is
  a test fixture, so `@unit/create` cannot succeed on the shipped server.
- **Terrain.** Per-cell `Cell{class, elev_cm, cover, room}`, 18 terrain classes, run-length tiles;
  `LocomotionTable` from `mush-atlas/content/locomotion.json`; bounded 8-connected A\*
  (`DEFAULT_PATH_MAX_CELLS = 4096`; **`path()` has no production caller**, `+move` steps cell by cell).
  Windows composite tiles; the binding bound is `MAX_TILE_SIDE = 256` (`cells.rs:17`), so today's
  largest arena is 256 by 256 cells: 2.56 km square at the ground driver's 10 m cells, 6.4 km at the
  largest registered resolution (2500 cm). The per-cell `cover` byte is written as zero everywhere;
  cover in traces comes from the terrain class. **No spatial index anywhere**; occupancy is a linear
  scan.
- **Traces.** Bresenham `line_of_sight`, `direct_fire_path`, `indirect_fire_path`, `beam_path`, and a
  separate `hearing`, all integer, all walking the whole line. The rich `Trace` (cover, crossed units,
  blocker identity) reaches only the staff `@atlas/los`; combat collapses it to
  `MetricGeometry{range_cells, occluded}`. **`target_cover` has no gameplay consumer.**
- **Movement.** `+heading` takes an `x,y,z` vector in `-1000..=1000`; orders persist and replace, but
  `StandingOrder` has no reader outside `tick.rs`. **Motion is never integrated.** Position changes only
  through discrete placement.
- **Combat.** `+fire <contact>` checks focus, a live `fire` channel grant, a promoted clock, contact
  ownership and a shared-window line; then resolves by `draw % 2 == 0` and emits a provisional boundary
  fact. **No weapon model exists** and no damage is ever written; `mush-sim` never calls
  `mush-tactics::apply`.
- **Perception.** `mush-propagation` runs detection every 2 s over at most 8 domains with
  `threshold = noise_floor + range_cells * medium.attenuation()` and one `DEFAULT_SIGNATURE = 12`; on a
  surface window that detects out to **three cells**. Contacts are viewer-scoped opaque ids with a
  typed lifecycle (`contact_observe|refresh|share|unshare|loss|reacquire`), owner-fenced so no real
  `ObjId` crosses, and `unit.status` is composed at read time as a genuine per-observer projection. There
  is no canonical event ledger, provenance chain or evidence trace. Map visibility is an ACL, not LOS.
- **Weather** models opted-in places with a `visibility` field that nothing reads.

### 2.5 AI, content, config, economy, rulesets

- **Decisions.** A durable, token-checked decision pipeline: capture on the loop, dispatch off-loop
  through a semaphore of 8 workers (ceiling 32), retries with typed failure kinds, fallback to `npc_idle`,
  the chosen offer applied as an ordinary authoritative action (`mush-core/src/decision.rs`,
  `mush-server/src/decision_dispatch.rs`, `decision_llm.rs`). Provider routing by model prefix, budgets,
  breakers and retries live in `mush-llm`. **`mush-npc` has no deterministic behaviour layer**: no
  behaviour trees, utility scoring, patrols or routines. `mush-bot` is an external SSH player, not an
  in-world controller.
- **Content.** `mush-content` packs with manifest, digest, `requires`, a per-pack latch, plan/apply
  with fingerprints (`@atlas/pack`), kinds `RulesDefinition | CampaignSeed | Region | Neighborhood | Site`;
  appliers registered by atlas, npc and tacsim. Bounds 1,024 entries and 1 MiB per pack.
- **Config.** `mush-config` sections include `[tacsim]`, `[npc]`, `[atlas]`, `[economy]`, `[wod]`,
  `[shadowrun]`, `[rulesets]`; no `[simulator]`. Modules never read TOML; `config_ext.rs` maps typed
  sections into module configs at composition. Deployment files `deploy/dev1`, `deploy/dev2`.
- **Economy data.** A currency or resource is already data: `InstrumentDef{id, ruleset, code, kind,
  scale, transferable, negative_policy, ..}` with exactly four kinds `Currency | Commodity | Requisition |
  Entitlement` (`mush-economy/src/record.rs:29,117`). TacSim declares no instrument and no price today;
  its packs carry `CommodityDef`, `RecipeDef`, `LocationDef`, `BodyDef`, routes and formations.
- **Rulesets.** `mush-ruleset` is the sheet schema, doc, store, chargen, render and vitals library plus
  a `Ruleset: GameModule` trait; exactly one ruleset composes per server (`modules.rs:723-776`).
  `ruleset-tacsim` is a plain `GameModule`, not a `[rulesets].enabled` id. Dependents: `ruleset-wod` and
  `ruleset-shadowrun` are used by `mush-server`, `mushd` and `mush-contract-tests`; `mush-ruleset` by
  seven crates including `mush-npc` and `mush-tactics`; `mush-dice` by `mush-tactics` and the two
  character rulesets. The WoD ST path's `call_structured` (forced tool, JSON fallback, one repair round
  trip, typed validation) is the structured-call spine doc 07 §5 says to copy.
- **Test fixtures.** `mush-testkit::TestWorld` wraps the real `Engine`; `mush-contract-tests`'
  `tactical_gate` rig installs units, placements, contacts, weapons leases and fire windows for Gate 2;
  full compositions use `mush_server::modules::server_modules()`.

### 2.6 The one-line reading

The substrate for the story exists (world task, store, leases, seats, focus, windows, traces, contacts,
decision pipeline, packs, definition registry). The three things the story needs on top of it do not:
integrated motion, a weapon and damage model that reaches `mush-tactics::apply`, and a perception model
with usable ranges. Everything else in this survey is about choosing the shape of those three so that
lasers, bipeds, sensors, ECM, ECCM and elevation are additions.

---

## 3. The acceptance, restated

Doc 16 is complete when both of the following pass on the production composition.

**The story test** (one test in `mush-server` or `mushd`, through `Inbound::Cmd` and the real
coordinator): two connected accounts create characters, walk from the OOC welcome hall into the arena,
pick opposite teams, board pods, receive default tanks, drive across terrain, see each other only after
line of sight and range allow, lock, fire cannons, and one tank is destroyed; the match records a result,
both pilots leave, and a second match can start. The test asserts the observer rule (a tank not yet seen
appears in no output to the other pilot) and the money rule (nothing in the campaign economy changed).

**The scale harness** (a test that runs in CI with thresholds; scripted reflex AI only, no LLM):
300 units placed and moving, 24 fighting, 150 weapons on their cadences, one full sensor pass per unit per
pulse-second, on one core, with per-pulse p99 under the budget chosen in Q2 and memory flat over an hour
of simulated time. The harness runs at the configuration maxima of every world fact in §9.

---

## 4. Questions

Each question gives the context, the options, a recommendation, what the recommendation forecloses, and
the configuration knob or harness assertion that pins it.

### A. Combat authority and time

**Q1. Where does a pulse run?**

Context. Doc 06 §6.3 rules that ticks are scheduled work inside the single-writer world loop. The Codex
branch followed that and paid 238 to 380 ms per 100 ms tick, but its cost came from whole-record
serialisation, same-tick replay and unbounded collision search, not from the topology. At the 1 s pulse
of ruling 2A, a 20 ms pulse is a 2% duty cycle on the world task. The tree already has an off-loop seam
(`ModuleAsync`) and a pure-function driver contract (doc 06 §6.7). What it lacks is a `step` hook.

- **A (recommended).** Keep §6.3. Add `step` to `DomainDriver` as the pure phase over an immutable
  snapshot. Split each pulse into (1) an off-loop *compute* phase (motion integration, perception pairs,
  scheduled-impact resolution, AI reflexes) run through `ModuleAsync` over the snapshot, and (2) an
  on-loop *apply* phase that validates the proposed commit against revisions and control epochs (the
  doc 07 §5 stale-rejection pattern) and commits one delta-only bundle. Escape hatch: if the harness
  shows the on-loop apply phase alone exceeding the Q2 budget, or if more than one region must be active
  concurrently at full load, move to B.
- **B.** One simulation task per active region owning in-memory state, its own writer lane, commands
  in and events out, the world task holding references. Isolates a stall to one region; costs a second
  writer, cross-cutting reads for custody, destruction and economy, and a new recovery story.
- **C.** Per-pulse whole-record authoritative bundles with in-pulse self-verification (the Codex shape).
  Rejected on the branch's own numbers.

Forecloses: nothing permanent; A can become B later because both use the same pure `step`.
Knob: `[tacsim.combat].pulse_budget_ms`. Harness: apply-phase p99 under budget; compute phase never on
the loop.

**Owner, 2026-09-16: leaning B.** Splitting the simulation per map now leaves less to unwind when
scale makes it a requirement; economy-impacting effects can still go to the world loop, combat
resolution should be local. The owner asked what B costs in reality. The cost sheet:

*What B needs that A does not.*

1. **A sim core as a library.** `Arena{poses, unit cells, contacts, orders, scheduled impacts}` with a
   pure `step(dt, intents, rng) -> StepOutput{events, frames}`; the region task is a thin tokio wrapper
   around it. Tests and the harness drive `step()` synchronously with no task at all. Needed under A
   as well (it is the driver's `step`), so not B-specific. About 2 to 3k lines either way.
2. **Custody transfer.** At match start the world task checks the units and their pilots out to the
   arena: the unit records are marked `deployed:<domain>`, ordinary mutations refuse, and a snapshot is
   handed to the task. At `Ended` the task hands back the final state and the world task applies it as
   one ordinary authoritative action. B-specific. About 500 to 800 lines plus one contract test that
   the arena never writes a world-owned key.
3. **Command routing.** Cockpit verbs validate focus and lease on the world task, where authority
   stays, then forward the intent to the arena's bounded inbox (latest-wins for continuous, sequenced
   for discrete, doc 06 §6.5). B-specific, about 300 lines; the `+fire` selector shape already exists.
4. **Output.** The arena resolves audiences to its pilots' connections and transmits through the same
   `GatewaySink`. It needs a read-only view of pilot-to-connection (the session manager's attach
   records) and its own per-observer frame builder. B-specific, about 500 lines. The risk is
   duplicating the semantics of `resolve_classified_sends` (spoof, ignore, `can_perceive`); scope the
   arena to cockpit frames and notices only, and leave say and pose inside the pod as world output.
5. **Lifecycle.** Spawn, drain with a bounded timeout in both shutdown tails, `catch_unwind` around
   `step` so an arena panic aborts that match and the world continues. About 200 lines; the first such
   task in the tree, since no task registry exists.
6. **Durability.** With Q3 C, none: the arena never touches the store, and the world task writes the
   result and the flushed ledger at `Ended`. When IC play needs Q3 A, the arena gets its own writer lane
   with disjoint key ownership (the persist writer already merges lanes in one acceptance order). That
   is the one expensive piece, and it is deferred.
7. **Cross-cutting readers.** While a unit is deployed, its `unit.status`, contact panel and map frame
   come from the arena; the world's own views show "deployed" and nothing more. Economy, custody and
   destruction are untouched because custody-in plus custody-out means the world only ever sees the
   before and the after.

*Total B-specific plumbing:* roughly 1.5 to 2.5k lines over A, most of it the custody protocol and the
output path. *What B buys:* isolation (a stalled or panicking arena costs one match), parallelism
(arenas on their own cores; the world loop's duty cycle unaffected), a per-arena pulse rate (Q2 B
without touching the domain clock floor), and a process boundary already drawn for the day an arena
moves out of process. *What B costs permanently:* two places where "where is unit X and how hurt is
it" can be answered, reconciled only at custody boundaries; every future feature that wants combat and
world state in one transaction (salvage on the spot, mid-fight ammunition economics) must be designed as
an arena event plus a later world action, never a joint write.

*Shape if B:* the arena is the single writer for its own facts while a match runs; the world task is
the single writer for everything else; the seam is custody in, intents in, events and frames out,
custody out. Never a shared lock. Doc 06 §6.3 is amended to "one writer per authority; the arena is an
authority".

**Q2. Pulse rate and budget.**

- **A (recommended).** 1 s pulses (ruling 2A's lower bound) for the first builds; budget 50 ms p99 for
  the apply phase at harness scale. Text and TUI output are paced by the pulse; there is no client to
  interpolate for.
- **B.** 500 ms pulses. Feels livelier; halves the budget; requires lifting the domain clock floor
  (`MIN_PULSE_PERIOD_MS = 1_000`) and the whole-second flooring in `tick_at`.
- **C.** 250 ms or faster. Not needed by any surface we ship.

Forecloses: B and C are config changes later if A holds. Knob: `[tacsim.combat].pulse_ms` with a floor
asserted by the harness.

**Owner, 2026-09-16: B (500 ms).** A middle ground between the 100 ms experiment and the one to two
second pulse; livelier, and with W14's one-second cycle floor the resolution has to sit below the
cycle. Under Q1 B the arena's pulse is its own clock, so the `mush-sim` `DomainClock` floor of one
second stays for world-side domains and needs no change. Scheduled impacts carry millisecond times and
are applied in timestamp order within the pulse they fall in; a one-second weapon fires at most every
other pulse. Budget: 25 ms p99 per arena pulse on one core at harness scale (a 5% duty cycle).

**Owner, 2026-09-16, refinement: the pulse rate is adaptive per arena.** Many maps may be running with
mostly NPC units on them while a player or two walk across; those run at 5,000 ms. When a threshold is
crossed the arena drops to 1,000 ms, and during active multi-unit conflict to 500 ms, "and it all works
the same way regardless". Consequence for doc 16: the simulation must be rate-independent (motion
integrates over actual elapsed milliseconds, cadences and cycle times are in milliseconds, scheduled
impacts are by arena timestamp, draws are per event not per pulse), with three named tempo states
(`Quiet` 5,000 ms, `Alert` 1,000 ms, `Engaged` 500 ms), config thresholds and hysteresis, and the tempo
changes recorded in the match ledger.

**Q3. What is durable during a fight, and how.**

Context. Every pulse commits a `WriteBatch`; the boot path scans `obj:` and per-namespace module docs.
Whether `SpatialState` persists placements as one document or per key decides whether 300 moving units
re-serialise one blob per pulse (§5 fact F1).

- **A (recommended).** Poses and unit cells commit per pulse as delta records keyed per unit (never one
  whole-domain document); fire, hit, damage, disable, destroy, spawn and despawn append to one
  `mush_core::Ledger` per match (`log:combat.<domain>`); a whole-state snapshot is written every N
  pulses or on match end for cold reads. The ledger is the in-world black box (§9).
- **B.** Everything per pulse as whole records (rejected, see Q1 C).
- **C.** Nothing durable until match end; RAM only. Simplest; violates C11 and loses the record on
  a crash.

Knob: `[tacsim.combat].snapshot_every_pulses`. Harness: bundle bytes per pulse bounded and roughly linear
in units that moved.

**Owner, 2026-09-16: C for OOC matches.** An OOC match is allowed to fail on a crash. Amended shape:
the arena keeps everything in memory; the match ledger (fire, hit, damage, disable, destroy, spawn,
despawn) is an in-memory append log that is flushed with the result at `Ended` through the world task
as one ordinary action, so the black box still exists for debugging and for the story test's
assertions. Q3 A (per-pulse deltas and snapshots) becomes a stated requirement before IC play, together
with Q4 B. Combined with Q1 B this is what makes B cheap now: the arena task needs no store access.

**Q4. Crash and restart policy for a running match.**

- **A (recommended).** Void the match: on boot, any domain found `Running` is closed, pilots are
  released to their team rooms, the match is recorded as aborted, and pods unlock. This is the branch's
  SI-D5 and doc 10 3A's "recovery floor" spirit; it is also literally what a simulator does.
- **B.** Resume from the last snapshot plus the ledger tail. Needed later for real-world fights (doc 10
  §10); not for the arena.

Forecloses: nothing; B is additive because Q3 A keeps the inputs.

**Owner, 2026-09-16: A now; B is a requirement before IC play.**

**Q5. Arithmetic.**

- **A (recommended).** Authoritative state in integer centimetres and millidegrees, velocities in
  cm/s, times in pulse counts and milliseconds; f64 permitted inside a pulse's pure computation as long
  as results are quantised before commit and every draw comes from the recorded RNG (doc 06 §6.6, doc 09
  C5). Replay is for debugging, not correctness (§1.1 item 5: no client, no lockstep).
- **B.** Exact rational arithmetic throughout (the branch). Rejected: it bought sub-microsecond
  determinism nobody can observe at the cost of the tick.

Harness: two runs from one seed produce one state hash.

**Q6. The single-writer invariant and other modules.**

Context. Economy, custody, destruction and scenes all read spatial truth from the world task. Under Q1 A
nothing changes. Under Q1 B every such reader needs a projection.

- **A (recommended).** Under Q1 A, combat writes through the ordinary bundle and other modules keep
  reading the world. Declare now, in doc 16, the list of world facts combat is allowed to write
  (poses, unit cells, contacts, match records, the combat ledger) so the escape to Q1 B has a bounded
  surface.

### B. Scale and the harness

**Q7. The numbers.**

- **A (recommended).** As in §3: 300 moving, 24 fighting, 150 weapons, one sensor pass per unit per
  second, 50 ms p99 apply on one core, flat memory over an hour. These exceed the owner's "100+ moving,
  12v12 fighting" so that the production numbers sit inside the tested envelope.
- **B.** Match the owner's numbers exactly (100 moving, 24 fighting). Cheaper to reach; leaves no
  headroom.

Knob: harness constants in one file. Assertion: the harness fails CI, not just logs.

**Q8. Where the harness lives and when it runs.**

- **A (recommended).** A `#[ignore]`d integration test in the game crate driven by a CI job on every
  push to `main` and on the combat PRs, with a JSON report kept as an artifact; locally runnable with
  `--ignored`. Reuse the `H3` scale-harness plan in doc 11-h where it overlaps.
- **B.** A Criterion benchmark. Good for micro-costs, poor for a whole pulse with I/O.

**Q9. What the harness must model to be honest.**

- **A (recommended).** Real terrain (a generated 256 by 256 relief), real windows, real leases and
  focus for the 24 fighters, the real store in memory mode, scripted reflex AI driving the other 276
  through ordinary intents, and the real output fan-out to at least 12 connected fake sessions. What it
  may stub: the LLM (always), Redis (memory store), the SSH transport.

**Owner, 2026-09-16: yes, stub the LLM by default.** Added:

**Q9b. The cost run.** An `#[ignore]`d harness mode that is never run by CI and starts only at the
owner's direction: it requires an explicit flag (`TACSIM_LLM_COST_RUN=1`), the OpenRouter key from
the environment, and a hard USD cap per run. It turns on the deliberation layer (Q38) for N minutes of
harness play and reports calls, tokens and USD per unit-minute, per squad and per match, using
`mush-llm`'s existing budget and audit records, as a table doc 16 can cite as the cost model for the
LLM side of the design. Recommended: build it with the harness, run it once per major AI change.

### C. Spatial model

**Q10. Coordinates and units.**

- **A (recommended).** Keep the two-layer shape the tree has (metric poses, centimetre windows) but
  make combat's authoritative pose integer centimetres with millidegree heading, converting to the f64
  `Pose` only at the placement boundary. `z` is carried from day one and pinned to the cell's `elev_cm`,
  which is zero on the first arena. Elevation later is a heightfield import (doc 14 §3.5) plus the slope
  term already in `move_cost` and the elevation profile already in traces.
- **B.** Rewrite `Pose` as integer. Cleaner; touches every placement consumer; not needed for the story.

Forecloses: nothing. Knob: `cell_cm` per driver (1000 for the arena).

**Owner, 2026-09-16: B.** Floats are a smell to fix before building on them. Shape:
`Pose{x_cm: i64, y_cm: i64, z_cm: i32, heading_mdeg: u32 (0..360_000), posture}`. The atlas already
has `PointCm`, `RectCm` and `cm()`; windows are already centimetres; authored packs keep metres in JSON
as human input and convert at load. The cost is the conversion sites: `pose_at` and `pose_cm` in
`mush-atlas/src/window.rs`, `Transition::Place`, `+move`, journeys, the map commands, and their tests.
Sequenced as the first item of §7 step 3, before any motion code exists to port.

**Q11. Arena size and the window cap.**

Context. `DEFAULT_WINDOW_MAX_CELLS = 65,536` gives 2.56 km square at 10 m cells; over-cap windows coarsen
rather than refuse. A regional map with key spots for patrols wants more.

- **A (recommended).** Ship the story on one 2.56 km arena. Doc 16 specifies the path to 10 km by
  raising the cap for the ground driver with a per-driver maximum and a spatial hash (Q13), and
  re-runs the harness there before any content depends on it.
- **B.** Multiple windows per region joined by portals. Complex; conflicts with "combat never straddles
  two regions" (§9).

Knob: `[tacsim.ground].max_window_cells`.

**Owner, 2026-09-16: A.** 2.56 km is fine for the story; the path to 10 km stays in doc 16.

**Q12. Terrain content for the arena.**

- **A (recommended).** One authored arena pack with roads, rough ground, light woods, water and a few
  buildings, using the existing 18 classes and cover values, no elevation, entry points for each team,
  three or four named key spots. Shipped through the content latch like the OOC area; the Codex
  branch's three-room centre content is the reference for the rooms and pods.

**Q13. Spatial index.**

- **A (recommended).** A per-window uniform grid (cell-bucket) index over placements, rebuilt each
  pulse in the compute phase, used for range culls in perception, impact candidate selection and
  occupancy. Not a quadtree; the map is a grid already.
- **B.** None; keep linear scans. Fine for the story, fails the harness.

Harness: perception pairs evaluated per pulse is O(units within max sensor range), asserted by count.

### D. Unit model

**Q14. Reuse M32 as the unit truth.**

- **A (recommended).** Yes, unchanged: sections, facets, cells, components, capabilities, the seven
  verbs, refuse-not-clamp, the journal. The stock tank is a `UnitSpec` with sections `hull`, `turret`,
  `left_track`, `right_track`, `engine` (five to start, within the owner's four to ten), each with
  `armor` facet cells and an `internals` section cell, components `cannon`, `machine_gun`, `visual_sensor`
  with their own cells (ammo, cycle). Bipeds later are a different `UnitSpec` with more sections and a
  different volume, not a new pipeline.
- **B.** A combat-specific unit type. Rejected: it is the Codex duplication in miniature.

Forecloses: nothing. Note the missing structural verbs (install and remove a component) are not needed
for the story and stay out.

**Q15. Hit-location attribution.**

- **A (recommended).** By relative bearing at impact into a facet (`front`, `left`, `right`, `rear`,
  `top`) with a per-unit table mapping facet to the sections it can strike and their weights. World
  fact: modular plate armour by facet (§9). Exact impact geometry is never computed.
- **B.** Geometric volumes per section (the Codex rotating-box tier). Deferred until a case needs it.

Knob: the facet table lives in the `UnitSpec`.

**Q16. Motion integration.**

Context. Nothing integrates motion today; `nominal_speed_cms` is the one magnitude the engine knows.

- **A (recommended).** Kinematic integration in the pulse: desired heading and speed (latest wins) are
  clamped by the unit's `MobilityProfile` (max speed, acceleration, turn rate as mode rules the game
  config supplies), stepped along the heading with `move_cost` scaling speed by terrain class, blocked by
  impassable cells and by occupied cells, stopped at the map edge. Sub-cell positions are kept in
  centimetres. No rigid bodies, no ramming damage, no sand.
- **B.** Cell-hopping (move one cell per pulse when a cost budget fills). Simpler, reads badly at 10 m
  cells and 1 s pulses.

Harness: 300 units integrate in well under the compute budget.

### E. Weapons and damage

**Q17. Projectiles.**

- **A (recommended).** Scheduled impacts. On an accepted `FIRE`, resolve the shot immediately against
  the target's current pose and motion (Q18), schedule the impact at fire time plus flight time
  (range over muzzle speed, both from the weapon definition), and re-validate at impact: target still
  within the predicted cell neighbourhood and still alive, line still clear. Lasers later are the same
  with flight time zero and `beam_path` instead of `direct_fire_path`. Missiles and drones later are
  units, not projectiles.
- **B.** Projectile entities stepped per pulse with collision. Rejected for the story; the branch's
  largest cost for no gameplay gain at this fidelity.

Knob: per weapon `flight_time_per_cell_ms`, `revalidate_at_impact: bool`.

**Q18. To-hit resolution.**

- **A (recommended).** One recorded draw (doc 09 C5) against a target number from: base accuracy of the
  weapon by range band, the target's cover from the trace (`target_cover` finally gets a consumer),
  relative motion, and the pilot's gunnery rating, which is a placeholder 1 read through a
  `PilotProfile` seam so character skills can replace it without touching `FIRE`. The curve and its
  modifiers live in the weapon and game config; the draw and its recording are engine.
- **B.** Deterministic hit at range with no roll. Rejected: it removes the pilot from the loop.

Knob: `[tacsim.combat].resolution = "2d6" | "d100"`; a single game mechanic (Q32).

**Q19. Ammunition, cycle time, heat.**

- **A (recommended).** Cannon: one shot per accepted `FIRE`, `ammo` cell decremented on acceptance,
  `ready_at` pulse recorded (cycle time from the weapon definition), refusal while cycling. Machine gun:
  one burst per `FIRE` of k rounds resolved as k draws in one pulse, its own ammo and cycle. Heat: a
  `heat` cell and a dissipation rate exist in the definition schema from day one but no stock weapon
  produces heat until lasers arrive.
- **B.** Continuous automatic fire while a trigger is held. Rejected by the Compact world fact and by
  the event-rate bound.

**Q20. Damage, disable, destroy.**

- **A (recommended).** Damage reduces the struck facet's `armor` to its floor, overflow reduces the
  section's `internals`; a section at zero internals sets `SetSectionCondition(destroyed)` and applies
  the section's consequence from the game config: `engine` destroyed disables motion
  (`RevokeCapability(tactics.mobility)`), a track destroyed halves speed and blocks turning toward it,
  `turret` destroyed removes the turret weapons, `hull` destroyed destroys the unit. "Blows up" is hull
  destruction. Disabled units stay on the map as obstacles and contacts until the match ends. All of it
  is one `UnitMutationPlan` per impact.
- **B.** Hit points per unit. Rejected: it throws away M32 and forecloses bipeds.

Ruling 3A holds: disabled precedes destroyed; capture and salvage are outside the arena.

### F. Perception

**Q21. Replace the detection formula.**

Context. Today's threshold detects three cells on a surface window with the single default signature.

- **A (recommended).** Keep `mush-propagation`'s contact ledger, opaque ids, lifecycle and viewer
  fencing. Replace the threshold with: a sensor on the observer with a class and a max range, a signature
  on the target by class and posture, `line_of_sight` from the atlas through the shared window, and a
  classification delay (a contact is `unknown` for N seconds after first detection, then `classified`).
  Ranges and delays are config. Visual is the only sensor class in the story; a second sensor class is
  a config row plus, at most, one new trace kind.
- **B.** Keep the linear model and tune constants. Rejected: it has no per-sensor range and no
  classification.

Knob: `sensors.<class>.max_range_cells`, `signatures.<unit_class>`, `classify_after_s`.

**Q22. Cadence and interest management.**

- **A (recommended).** One pass per unit per second, staggered across pulses by unit id, range-culled
  through the Q13 index, LOS cached by cell pair for the pulse. Detection loss uses the existing
  refresh and TTL bands.

Harness: pairs evaluated per pulse asserted.

**Q23. What reaches a pilot.**

- **A (recommended).** The cockpit map frame is built from the platform's own contacts and known
  terrain, never from room visibility (the `may_see` ACL is not consulted for the arena). `CONTACTS`,
  transition notices (`acquired`, `lost`, `classified`) once per transition with hysteresis, `STATUS`
  and `WEAPONSTATUS` from `unit.status` and the unit view. The web and TUI get the same frames.

**Q24. ECM and ECCM later.**

- **A (recommended).** Design the sensor row now with `emission` and `jamming` fields left null; ECM is
  a component that raises the observer's effective noise for a sensor class within a radius, ECCM a
  component that subtracts from it. Both are content plus one term in the Q21 comparison. The doc 09
  "jammed is still seen" rule (R4) is revisited then, not now.

### G. Control and cockpit

**Q25. Pods, seats, focus.**

- **A (recommended).** A pod is a Thing that is an interior space; boarding is `Transition::Place
  Interior{space: pod, slot}`; the tank is the unit; the pod's station is assigned to the boarding
  character, `request_seat_activation` grants the `helm`, `sensors` and `fire` channels, and the
  account's focus moves to the station. Leaving reverses it. The Codex branch's centre content is the
  reference for the rooms and eight pods.

**Q26. Verbs.**

- **A (recommended).** Bare cockpit verbs dispatched contextually while focused on a station:
  `STARTUP`, `HEADING <deg>`, `SPEED <kph>`, `VIEW`, `CONTACTS`, `LOCK <contact>`, `FIRE [<weapon>]`,
  `WEAPONSTATUS`, `STATUS`, `EXIT`. Each is a selector that stages one intent, exactly as `+fire` is
  today; the `+heading` vector form is retired for ground units. Lobby verbs: `SIM TEAM red|blue`,
  `SIM LEAVE`, `SIM START` (staff or either team lead), `SIM ABORT`.
- **B.** Keep the `+` prefixed generic verbs. They survive for non-cockpit domains (ships, later).

**Q27. Startup.**

- **A (recommended).** A real ten-second startup sequence with a `power` capability and a due pulse,
  during which movement and fire refuse; repeated `STARTUP` is idempotent. It is the smallest thing that
  makes "units begin offline" true and gives the AI reflex layer its first state.

### H. Match lifecycle

**Q28. The state machine.**

- **A (recommended).** `Idle → Selection → Boarding → Running → Ended → Idle`, with `Aborted` reachable
  from every state except `Idle`. Two teams, two to eight slots, no tonnage or value budgets, no terms or
  consent revisions, default tank per slot, AI fills nothing in the story (the second player is
  required). `Ended` is team elimination or `SIM ABORT`; reset releases every seat, lease, focus and pod
  atomically and can be re-run safely.
- **B.** The Codex roster (budgets, consent, reopen). Deferred to a later lobby doc.

**Q29. Disconnects during a match.**

- **A (recommended).** Doc 07 §1 applies unchanged: a dropped pilot's tank continues its last
  continuous orders, fires nothing, and is reclaimable on reconnect; after the D1 grace the tank is
  treated as idle. In the story a disconnect ends nothing; `SIM ABORT` is always available to the other
  pilot. AI takeover of a human slot is Q36's business, later.

### I. Config versus Rust

**Q30. The game crate and the content directory.**

- **A (recommended).** Rename `ruleset-tacsim` to `tacsim` (the game crate). It holds the composition
  root (moved out of `mush-server/src/modules.rs`, which keeps only host wiring), the game-specific Rust
  (weapon resolution, damage consequences, the reflex AI, cockpit verbs, the arena driver), and
  `content/` with the packs: `units/*.json`, `weapons/*.json`, `ammunition/*.json`, `sensors/*.json`,
  `resources.json` (the ten families as `InstrumentDef`s), `arenas/*.json`, plus the existing campaign
  seeds. Engine crates keep no game data; `mush-atlas/content/locomotion.json` stays because it is engine
  vocabulary.
- **B.** Keep the ruleset name and shape. Rejected: the review found it had already become the
  composition root, which is only wrong while the name says "ruleset".

**Q31. Definitions through the C1 registry.**

- **A (recommended).** Every unit, weapon, ammunition, sensor and consequence table is a
  `RulesDefinition` entry with an exact `DefinitionRef`, frozen at composition, validated with its
  references. Persisted units point at exact versions (already true for `UnitSpec`). Changing a stock
  tank is a new version and a migration chain, never an in-place edit.

**Q32. What stays Rust.**

- **A (recommended).** The closed vocabularies: mutation verbs, `CellAddr` arms, instrument kinds,
  section consequence kinds (`disable_motion`, `slow`, `remove_weapons`, `destroy_unit`), trace kinds,
  sensor comparison terms, order channels, match states, the one resolution mechanic. Config picks from
  these by name; a config file that needs a conditional is a sign the vocabulary is missing a verb, and
  the verb is added in Rust (or, for narrative scripting only, in Rhai).

**Q33. The one resolution mechanic.** *(Owner, 2026-09-16: not tied to any dice system; "limiting
ourselves to dice in a game the players never see a roll in seems smelly." Options expanded; open.)*

Context. Doc 09 C5 requires recorded randomness and doc 10 2A requires resolution from recorded
intents and RNG; neither requires dice. Every option below is one recorded draw of a uniform integer;
they differ in how the target is computed and what the pilot can be told.

- **A. Dice-shaped curves** (`2d6 + rating >= target`, d20, d100, pools). Integer, familiar to
  tabletop players, easy to hand-tune. Against: coarse steps (a 2d6 modifier is a 3 to 14 point
  swing), modifiers stack additively whether or not that makes sense, and the shape is a tabletop
  artifact nobody in the cockpit will ever see.
- **B. An explicit probability model.** Compute a hit probability in per-mille from a designed
  function (a logistic in "logit space" so modifiers compose multiplicatively: range band, target
  cover, relative motion, gunnery, sensor quality), draw `u` in `0..1000`, hit if `u < p`. Integer
  throughout. Smooth, tunable, and the number is showable: `WEAPONSTATUS` can print "63%". Against:
  the curve must be designed rather than borrowed, and the per-mille granularity must be chosen once.
- **C. Geometric dispersion.** Derive the probability from geometry: the shooter's angular error cone
  (gunnery, motion, range, weapon) against the target's presented angular size (silhouette by facet,
  reduced by cover). Two small draws (bearing and elevation error) or one against the resulting area.
  Emergent and honest: distant and small targets are hard because they are small, bipeds present more
  than tanks, cover shrinks the silhouette. Against: needs silhouette numbers per unit and facet in
  config, and balance is indirect (you tune cones and sizes, not percentages).
- **D. No randomness.** Hit chance becomes expected damage: a cannon shot always lands for
  `p × damage`. Predictable and cheap; removes variance and drama, and makes a single decisive shot
  impossible. Defensible for machine-gun bursts (many rounds), wrong for cannons.
- **E. Opposed checks.** Attacker versus a defender evasion value, either as A or B. Adds a pilot
  decision on the receiving side (evasive driving). Can be layered onto B or C later as a modifier.

**Recommended:** B as the mechanic and C as the source of its inputs. The per-mille probability is
computed from a dispersion cone against a presented silhouette, with cover and motion as explicit
terms, so the number has physical meaning and bipeds, lasers and sensors change inputs rather than
the mechanic; one recorded draw resolves it; the cockpit shows the percentage before the shot. Damage
is fixed per ammunition type (no damage roll) and location is by the Q15 facet table. Machine-gun
bursts resolve k rounds as one binomial draw against the same p to keep event counts bounded. Call it
a resolution, not a roll; `mush-dice` keeps only the expression evaluator for `+roll` and scripts.
The one thing to decide now is the granularity (per-mille) and that the curve lives in config as a
named table, not in code.

**Owner, 2026-09-16: the recommendation is accepted.** Per-mille probability from a dispersion cone
against a presented silhouette, one recorded draw, the percentage shown in the cockpit, fixed damage
per ammunition type, facet hit location, bursts as one binomial draw; the curve is a named config table.

### J. Ruleset removal

**Q34. Delete the character rulesets, and when.**

Context. `ruleset-wod` and `ruleset-shadowrun` are used only by `mush-server`, `mushd` and
`mush-contract-tests`; deleting them touches `modules.rs` (`server_modules_with`, `ruleset_modules`,
`vendor_caps`, `market_catalog_sources`, `market_availability`, `market_delivery`, `econ_observers`,
`housing_title_issuer`), `config_ext.rs`, the WoD `ModuleAsync` arm, four `mush-config` sections, `mush-market/src/record.rs`, four contract tests, four `mush-server` tests, eleven `mushd` end-to-end tests, the two crates' own suites, the `wod_roll`/`sr_roll` grounding abilities, two K8.4 authoritative providers, and docs 04, 08d, 08f, 08g plus the example configs.

- **A (recommended).** Delete them in one cut *before* the combat work, because the story test composes
  the production root and the root must stop taking `OnboardingConfig`/economy configs from crates that
  no longer exist. Order: hoist `call_structured` and the structured-tool machinery from
  `ruleset-wod/src/st.rs` into `mush-llm`; make `mush-ruleset` the platform's sheet library with one
  implementation (the game's) and drop `[rulesets].enabled`; move housing's title issuer and the
  market's catalog, availability and delivery sources onto game-owned implementations or delete the
  Shadowrun-only ones; delete the two crates, their configs, tests and docs; re-bless the contract gates.
- **B.** Delete after the story lands, composing the story beside WoD meanwhile. Keeps two sheet
  schemas and two economies alive through the whole combat build.

**Owner, 2026-09-16: A, explicitly.** Delete the rulesets first and clean the base before stacking
cards on it.

**Q35. The character sheet for the 2350 game.**

- **A (recommended).** Keep `mush-ruleset`'s schema, doc, store, chargen and vitals as platform; define
  one game sheet with the pilot ratings (`gunnery`, `piloting`, `sensors`, placeholders at 1), the
  character-first facts doc 10 §17 to §19 already decided, and nothing else until the economy loop
  needs it. `+sheet`, `+chargen` and `+roll` survive as game verbs over that one schema.

### K. Testing and CI

**Q36. The story test and the gates.**

- **A (recommended).** One end-to-end test as in §3 in `mushd/tests` (the all-in-one) and its twin in
  `mush-server` through the gateway boundary; the harness of Q7 to Q9; every existing behavioural anchor
  listed in §2 kept (`s9_fire_gate`, the M32 atomicity tests, the existence-hiding tests, the ground
  engagement composite, the doorway traces); the panic budget and state-slot gates re-blessed in the
  same commits that change them, never left red.

### L. AI pilots

**Q37. The reflex layer.**

- **A (recommended).** A deterministic controller in the game crate over engine queries (`path`,
  `line_of_sight`, the contact ledger): startup, follow authored waypoints, acquire the nearest
  classified hostile, close to the weapon's best band along the path, fire when ready, break off at a
  damage threshold, resume patrol. Implemented as a small state machine per unit and a utility score for
  target choice, run in the pulse's compute phase, submitting ordinary intents through the same admission
  as a player (the branch's demo AI shows the shape). It is what makes the harness's 276 non-fighting
  units move. Doc 07 §4 already assigns it these responsibilities; today `mush-npc` has none of it.

**Q38. Deliberation.**

- **A (recommended).** The existing decision pipeline, per squad or commander rather than per unit,
  event-driven with a floor of 30 s between requests, emitting orders with a time to live through the
  same admission; fallback is the reflex doctrine, not `npc_idle`. Span of control caps the units a
  commander may direct (§9). Not in the story; the hook is designed in doc 16 so the harness can turn
  it on later.

**Q39. Provider failure.**

- **A (recommended).** Nothing new: the pipeline's `Unavailable` retries, breaker and fallback already
  never block the loop. Doc 16 adds the visible in-world effect ("mind link dropped, unit on doctrine")
  and a per-match LLM cost cap.

### M. Scope of the first three follow-on additions

**Q40. The order after the story.**

- **A (recommended).** Lasers (Q17 A makes them a weapon row plus `beam_path`), then bipeds (a
  `UnitSpec` with legs, torso sections and a walking mobility mode; Q15's facet table does the hit
  locations), then a second sensor (thermal: a sensor class row and a signature term), then ECM and
  ECCM (Q24), then elevation (Q10 A already carries `z`; import one heightfield and re-run the
  harness). Each is a doc 16 §"extension test" that must pass without changing the story test.

---

## 5. Facts needed before doc 16

| # | Fact | Why | How |
|---|---|---|---|
| F1 | Does `SpatialState` persist placements per key or as one document? | Q3 A depends on delta-only pulse commits | Read `mush-sim/src/lib.rs` boot and `data.rs` persistence paths; count bytes per placement upsert in a test |
| F2 | Cost of `line_of_sight` on a 256 by 256 window, per pair, in debug and release | Q13 and Q22 budgets | A micro-benchmark over the existing trace with random pairs |
| F3 | Cost of one `sim.domain_pulse` today with 300 placements and no combat | The baseline the harness starts from | Synthetic placement fixture on `main` |
| F4 | Whether `TickCadence::Fast` composes with the durable schedule commit path outside the `tacseam` stub | Q2 B feasibility | Read `engine.rs` arming and `world_task.rs` driver; one test |
| F5 | Ledger append throughput at 100 events per second with 16 KiB chunks | Q3 A | Existing `mush-canon`/`mush-scenes` tests as the template |
| F6 | Line counts and test counts of `ruleset-wod`, `ruleset-shadowrun`, their tests and the dependent host tests | Q34 sizing | `wc` and `grep -c "#\[test\]"` |
| F7 | What `market`, `housing` and `mush-npc` actually need from `mush-ruleset` (the seven dependents) | Q34 and Q35 | Read each dependent's imports |
| F8 | Whether the OOC welcome hall content (`neighborhood.ooc_area.json`) on dev1 is at a version the latch can upgrade | The arena pack must reach existing worlds | Compare the applied latch on dev1 with the pack version |

---

## 6. Defaults taken unless vetoed

Answered 2026-09-16: **Q1 B** (arena task per map; cost sheet under Q1), **Q2 B, adaptive** (500 ms
engaged, 1,000 ms alert, 5,000 ms quiet; 25 ms p99), **Q3 C** (RAM only for OOC, ledger and result
flushed at `Ended`; Q3 A required before IC play), **Q4 A** (void; Q4 B required before IC play),
**Q9 A plus Q9b** (LLM stubbed; owner-directed cost run), **Q10 B** (integer `Pose`), **Q11 A**
(2.56 km), **Q33 B with C's inputs** (accepted), **Q34 A** (delete the rulesets first), and **W1 to
W16 ratified**.

Defaults standing as decided: Q5 A, Q6 A, Q7 A, Q8 A, Q12 A, Q13 A, Q14 A (five sections), Q15 A,
Q16 A, Q17 A (scheduled impacts), Q18 A, Q19 A, Q20 A, Q21 A, Q22 A, Q23 A, Q24 A, Q25 A, Q26 A (bare
verbs), Q27 A, Q28 A, Q29 A, Q30 A (`tacsim` game crate), Q31 A, Q32 A, Q35 A, Q36 A, Q37 A, Q38 A,
Q39 A, Q40 A.

The survey is closed. Doc 16 (requirements) and 16b (implementation plan) are written from it.

---

## 7. Sequencing, as the defaults imply

1. Ruleset removal cut (Q34, Q35, Q33): composition root in the game crate, one sheet, one mechanic,
   `call_structured` in `mush-llm`, gates green.
2. Harness skeleton (Q7 to Q9) over the substrate as it stands: 300 placements, the pulse, output
   fan-out. It fails its budget or passes; either way it is the baseline.
3. Integer `Pose` first (Q10 B), then the arena core library with its pure `step`, the arena task
   wrapper with custody in and out, command routing and the output path (Q1 B), motion integration
   and the spatial index (Q13, Q16). Harness re-run, now driving `step()` directly.
4. Stock definitions and the arena pack through the registry and the latch (Q12, Q14, Q30, Q31).
5. Pods, seats, verbs, startup, lifecycle (Q25 to Q28).
6. Perception replacement (Q21 to Q23). Harness re-run at full sensor load.
7. Weapons, scheduled impacts, resolution, damage (Q17 to Q20). The story test passes.
8. Reflex AI (Q37). Harness at 300 with all units driven.
9. Extension tests in Q40's order.

Each step ends with the story test and the harness green, the contract gates blessed, and the doc 16
ledger updated with what landed. No step introduces a second lane, a version ladder, or a fixture that
impersonates a production module.

---

## 8. Outline of doc 16

1. Purpose and the two acceptances (§3 here).
2. World facts the engine relies on (§9 here, as ratified), each with its knob and harness assertion.
3. The model: time and authority (Q1 to Q6), space (Q10 to Q13), units (Q14 to Q16), weapons and
   damage (Q17 to Q20), perception (Q21 to Q24), control (Q25 to Q27), lifecycle (Q28, Q29).
4. Records and definitions: the content packs, the registry entries, the per-pulse commit, the combat
   ledger, the snapshot.
5. Verbs and output: the cockpit and lobby surfaces on line, TUI and web.
6. AI: the reflex controller and the deliberation hook.
7. The game crate: composition root, config sections, what is Rust and what is config.
8. Tests: the story test, the harness, the anchors kept, the gates.
9. Extension tests: lasers, bipeds, thermal, ECM, ECCM, elevation.
10. Build cuts with acceptance per cut (§7 here).
11. What this document supersedes and the amendments owed to docs 04, 06, 10, 14.

---

## 9. World facts to ratify

Each fact bounds an engine cost. Ratify, strike, or amend; each ratified fact goes into doc 16 §2 with
its knob and its harness assertion, and into the setting docs beside the Retreat and Concordat material.

**Owner, 2026-09-16: all sixteen ratified**, with the conflict below read as recommended.

| # | World fact | Engine bound | Knob | Harness assertion |
|---|---|---|---|---|
| W1 | Standard ECM is good enough that no sensor detects at long range | Perception pairs per pulse | `sensors.*.max_range_cells` | Pairs evaluated is O(units within max range) |
| W2 | Lasers are excellent against drones and soft fliers, poor against armour at available power | No drone swarms; projectiles primary; tactical air absent | `units.*.class = drone` count cap per side | Unit classes on the map match the cap |
| W3 | Active sensors are suicide (anti-radiation munitions); doctrine is passive-only | A sensor pass is range plus LOS, nothing else | `sensors.*.active = false` | No emitter term in the comparison |
| W4 | Lasers burn optics in the open; fights happen in clutter at short LOS | Precomputed visibility pays; LOS cache hit rate | arena content | LOS cache hit rate above a floor |
| W5 | Transponders are spoofable; identification takes a close look | Classification is a slower cadence than detection | `classify_after_s` | Classification passes per pulse bounded |
| W6 | Tactical comms are LOS mesh jammed by the same ECM | Shared contacts fan out only within range | `comms.share_range_cells` | Share operations per pulse bounded |
| W7 | Licensed minds are leased over the relay; a dropped lease drops the unit to onboard doctrine | Deliberation is optional; reflexes always run | `[llm.routes.tactical_flash]` | Harness runs with the LLM disabled |
| W8 | The Compact caps span of control and requires a human trigger for lethal release | Bounded AI per commander; fire is discrete | `command.max_span` | Orders per commander per minute bounded |
| W9 | Heavy hulls are scarce post-Retreat; a company is a major commitment | Twelve a side is the setting's ceiling for a fight | `match.max_slots_per_side` | Fighters at 24 |
| W10 | The sky is a laser kill zone | No VTOL, aircraft or drone swarms on the tactical map; elevation is terrain, never flight | driver `media = ground` | No `air` medium participant |
| W11 | Contested ground comes in Interval pockets joined by corridors | Regions are domains; corridors are journeys; combat never straddles regions | `MAX_ACTIVE_DOMAINS` | One region per fight |
| W12 | Armour is modular plate by facet | Facet attribution; no impact geometry | facet table in `UnitSpec` | No geometry code in the impact path |
| W13 | Indirect fire needs a spotter net that ECM breaks | Artillery is strategic; direct fire only | none needed | No `indirect_fire_path` consumer in the story |
| W14 | Interval-grade power plants | One to fifteen second cycle times | `weapons.*.cycle_ms` floor | Shots per pulse bounded |
| W15 | Ammunition is logistics, not a number | Short engagements; units withdraw | `units.*.ammo` | Match length bounded in the harness |
| W16 | A battle's canon is reconstructed from black boxes | The combat ledger is an in-world artifact; replay from log is the recovery story | Q3 | Ledger complete for a harness match |

Conflicts to resolve while ratifying: W2 and W10 against doc 10 §8's combined-arms list (drones scout,
VTOLs give mobility, aerospace links atmosphere and orbit). Recommended reading: strategic and orbital
aerospace are untouched; tactical-map air is what the world fact removes, and "drones scout" becomes
"drones scout until a laser sees them", which keeps them as content later.

---

## 10. Not in this survey

The mission and contract loop (docs 08, 10 §5 to §7), campaign fronts and clocks, space combat, the
economy of ammunition and repair, capture and salvage after a fight, and the eight-slot lobby with
budgets and consent. Each of these consumes what doc 16 builds and none of them changes its shape.
