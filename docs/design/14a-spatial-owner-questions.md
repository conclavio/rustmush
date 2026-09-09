# 14a — The integrated spatial/room system: owner questions

**Answered 2026-09-09: the owner took every recommended option (1A–29A) and the §5 defaults, with
seven sharpenings recorded in §8; all binding. Doc [14](14-spatial-room-system.md) is written from
them.**

**Draft 0.1 — 2026-09-09.** The survey the owner answers before doc 14 — the design proposal for
one integrated spatial/room system — is written **from first principles** off the inputs in §1 and
the answers here, in the shape of [10a](10a-tacsim-e7-owner-questions.md) and
[08e](08e-e6-owner-questions.md): numbered questions, lettered options with a **recommended**
default, the fact each recommendation rests on, and a line on what the answer changes. Answer
`1A, 2B, …`, or `defaults` to take every recommendation; §4 lists the smaller choices doc 14 will
make on its own unless vetoed, and §5 asks for the facts (not decisions) the proposal needs.
Code grounded at `0bc5e92` (unchanged since); docs at `cd64419`.

Doc 11 and its [review](11a-neighborhood-geography-review.md) are **inputs, not constraints**: doc
14 derives the model from §1, not from doc 11's text, and supersedes doc 11 where they differ.

---

## 1. Givens — what is decided and will not be re-asked

From the owner (premises 1–6 of 11a, rulings R1–R4 of doc 11 §11, and the boot rule):

- **G1** Small, disparate, fully built neighborhoods reached by a travel verb; no world-scale grid.
- **G2** Bar-level and street-level tactical maps are part of a neighborhood's design; neighborhoods
  sit on a real grid relative to one another; the space between exists off-screen.
- **G3** Hydrate-on-demand: a player describes a fitting place, the agent builds it, they play
  there, a human who knows the neighborhood reviews it afterwards, it persists. Quotas are
  character-scoped caps on outstanding reviews (R3).
- **G4** Elevation and terrain are integrated so the map and the room system are one thing.
- **G5** No migration burden: concepts may be replaced outright; invariants that were expensive to
  get right are kept on their merits.
- **G6** Starfield is the reference for movement between scales — discrete bounded places at
  surface, orbital, system and interstellar scale; ships carry players between disparate
  locations; the space between is abstracted — for the Expanse-scale game and the land-based
  game alike.
- **G7** Worldgen digs; trust but verify (R1): the agent proposes everything a builder may create,
  a deterministic validator checks, humans review after activation where policy says so; a
  restriction whose only purpose is to keep an LLM from doing what a trusted builder may do is
  struck when it blocks the model.
- **G8** Briefs and manifests are JSON packs (R2).
- **G9** A starter area ships as the reference fixture in the default database (R4): static
  content, applied at first boot with no LLM call, never applied by a test instance unless the
  suite tests the starter area itself.

From the existing binding designs, kept unless a question below opens one: doc 06's single
authoritative placement graph and its transitions (§5, §7); doc 10's abstracted routes (1A),
journeys never pause for scenes (6A), carrier roots move their contained topology (C16A),
background journeys with opportunity events (C19A), interception promotes the carrier into a live
domain (C20A), one active focus per account (C21A); doc 09 C8's capability / lease / reservation /
revision split; doc 09b's typed-family lane rules; doc 03 §7's typed-action boundary (the model
emits typed records, never command text). **Q4 deliberately opens doc 06 §5.1's opaque pose.**

## 2. Grounding — what the tree has today

- **No geometry anywhere.** `DomainRecord { id, host, driver, status, clock, presentation_location,
  revision }` (`crates/mush-sim/src/data.rs`); the only driver `@domain/create` accepts is
  `sim.passive` (`commands.rs` `PASSIVE_DRIVER`); a pose is opaque JSON built from raw text
  (`SpatialParent::Direct { pose: RulesValue }`, `pose_from`); the only map is a 2×2 toy hex blob
  (`crates/mush-tactics/src/lib.rs` `toy_map`) behind `Ctx::set_map_blob`, which core never reads;
  `mush-propagation`'s `Geometry` is non-metric by design (range 0 or `ADJACENT_RANGE` = 2) and
  written to accept a real metric later. Nothing in the workspace is a coordinate, frame,
  transform, footprint, resolution, elevation, cover or line of sight; no `hexx` dependency.
- **The placement graph is real and authoritative**: `SpatialParent::{Direct, Interior, Attached,
  Unplaced}`, `Obj.location` a maintained projection (`graph::project`), interiors anchored
  `FixedDomain | MovingObject | Unanchored`, transitions planned then applied atomically
  (`transition::{plan, apply}`), `MAX_ACTIVE_DOMAINS` = 8, `DomainClock::{Passive, Pulse}`,
  `@domain/promote` with `PromotionTrigger::{PlayerPresent, CredibleContact, Interception,
  OperationActivated}`. No player verb enters a domain from a room.
- **Two journeys.** `route.rs::Journey` walks a plain object through a list of rooms on wall-clock
  seconds (staff `@route`, `+intercept`); `journey.rs::CarrierJourney` flies a carrier root over
  `RouteDef { endpoints: (ObjId, ObjId), legs: Vec<LegDef { to, base_s, hazard, interdiction }> }`
  on the IC clock with an idempotent leg cursor, `advance_due`, `reanchor` across rate changes,
  `MAX_LEGS` = 32, arrival into a domain host or interior (`arrival_parent`).
- **Rooms and exits are classic**: `ObjKind::{Room, Exit, Thing, Player, Npc}`; exits carry no
  distance or time; traversal is instantaneous and vetoed only through `HookPoint::MoveAttempt`
  (placement gate, timestop gate, housing access, hex lock); a zone is a Zone Master Object
  (`Obj.zone: Option<ObjId>`) carrying theme (`worldgen.theme`, resolved room → zone chain → `#0`),
  lock delegation, zone exits and weather (`docs/help/weather.md`).
- **Packs and the latch**: `PackManifest { id, version, kind, digest, requires }`, `pack_digest`
  over canonical JSON, `SeedPlan` → `seed::apply` with `SeedApplication.placed: BTreeMap<CanonicalId,
  ObjId>` in one `WriteBatch`, never re-run (`crates/ruleset-tacsim/src/{pack,seed}.rs`); pack
  locations are `LocationDef { label, role }` with no coordinates, created as `ObjKind::Room`.
- **Identity and hashes**: `CanonicalId` admits `[a-z0-9_.]`, 64 bytes; `DefinitionDigest`;
  `IdempotencyKeyText` (`[a-z0-9_.:/-]`); C8 `ReservationRecord`/`ReservationBook`
  (`crates/mush-core/src/{mechanics,action,resource}.rs`).
- **Clocks**: `ClockRef::{Utc, WorldIc, SceneTurn, DomainTick, Registered}`, `ClockStamp` with
  generation-checked arithmetic; `[game].ic_ratio` (IC seconds per real second, default 1) seeds
  generation one of the world timeline; `[tacsim.time].rate` is the campaign's own rate policy;
  scenes carry an `ic_floor`.
- **Boot and tests**: first boot seeds Limbo `#0` and God `#1` and writes through before serving
  (`crates/mush-server/src/persist.rs`); `[onboarding].start_room` defaults to Limbo with an
  optional `start_room_unapproved`; the testkit's `boot_world` is that minimal world plus core
  commands (`crates/mush-testkit/src/lib.rs`); the LLM double is `MockProvider`
  (`crates/mush-llm/src/mock.rs`); budgets are `BudgetScope::{Global, Feature, Actor, Npc}` with a
  `KillSwitch` at global / feature / NPC level.
- **Neighbors of the atlas**: `mush-faction::Region { zones: Vec<ObjId> }` with `ControlRecord`s;
  `mush-housing::Property { anchor, rooms, tier, landlord }`; `mush-scenes::SceneRecord { anchor,
  exclusive, participants, observers, … }`; the web client has a vitals panel and no map panel;
  generic panels render on web only (doc 06 §10).

## 3. The questions

### A. Scope

### 1. What doc 14 covers

**1A — the substrate, the packs, the apply path, the starter area, and the agent's contract
(recommended).** Doc 14 specifies: frames, elevation and terrain; rooms, exits and their geometry;
connections and the one travel system; tactical domains as windows onto the substrate; the pack
format and the apply/retire path; the starter area as its first pack; and the building agent's
contract (inputs, outputs, validator, budget, where it stops) — but **not** the survey UX of doc 11
§7, not presence (§6), not faction places (§9). Those become 14b/14c after 14 lands. *Rests on:*
G9 needs packs and apply in the same doc as the substrate, and G7's agent cannot be specified
without the validator the substrate defines. *Changes:* the doc's size and what cut 1 builds.
**1B — substrate only.** Cleaner, but the starter area then has no format to ship in. **1C — all of
doc 11 rewritten.** Too large to review in one pass; the survey and faction parts are separable.

### 2. First tactical consumer

**2A — street level first (recommended).** The first lattice driver is 1–2 m square/metric for
bar-level and street-level fights (WoD, Shadowrun, the starter area's coffee shop and back lot);
the TacSim ground driver (hex, tens of metres, mechs) samples the same substrate second, when E7
cut 8's pulse driver needs terrain. *Rests on:* G2 and R4 are street-level; TacSim's space combat
runs in orbital regions that are node graphs (Q5) and needs no terrain. *Changes:* the resolutions
the first tiles need and which driver lands first. **2B — TacSim ground first.** Puts the 30 m hex
in front of the coffee shop. **2C — both at once.** Two drivers before one map exists.

### B. What space is

### 3. What a room is on the substrate

**3A — rooms stay the unit of social play; geometry is attached (recommended).** A room keeps
`look`, `say`, exits, locks and contents; it additionally owns a **footprint** (a region of the
substrate on one floor). Tactical poses live on the substrate; *which room a pose is in* is derived
(the resolver doc 06 D1 promised). *Rests on:* every social system, NPC wake, scene anchor and lock
in the tree is room-keyed; changing that is not what G4 asks. *Changes:* nothing about MUSH play;
everything about how a room is authored. **3B — geometry primary, rooms derived labels over cells;
audiences by radius.** Rewrites every social command and NPC wake for no gain at bar scale.
**3C — rooms indoors, continuous outdoors.** Two models; the street is where scenes happen.

### 4. The coordinate substrate (opens doc 06 §5.1)

**4A — one metric substrate per region; the canonical pose is metric; lattices are per-domain
samplings (recommended).** Region-local metres, `+x` east, `+y` north, `+z` up; a pose is `{ x, y,
z, facing, posture }`; a domain declares a lattice (`Square { size }`, `Hex { size }`, `Metric`) and a
transform, and drivers work in lattice cells they derive from the substrate. Doc 06 §5.1's opaque
`RulesValue` pose remains only for driver-private posture. *Rests on:* G4 (elevation is a field
over a plane, not over hexes) and G2 (neighborhoods on a real grid). *Changes:* every driver reads
coordinates the same way; hex maths becomes a sampling, not an owner. **4B — hex-native regions,
metric derived.** Buildings become hexagonal and bar fights coarse. **4C — per-domain coordinates
only, no shared frame.** Rejected by G2 outright.

### 5. Where the metric map ends

**5A — metric surface regions, node graphs above them (recommended).** A surface region is a
bounded metric frame (order of tens of kilometres square) on a body; a body may have several
disjoint surface regions, each optionally tagged with a geographic reference; orbital, system and
interstellar "regions" are node graphs of locations and timed connections with optional corridor
geometry; ship and station interiors have local frames anchored to their carrier. Implicit space
*between* neighborhoods inside a surface region is coordinate-bearing (checkpoints and encounters
need a point to stand on); space between bodies is not. *Rests on:* doc 10 1A and G6. *Changes:*
whether a landing site has coordinates (yes) and whether an orbit does (no — it is a node).
**5B — a spherical frame per body.** Lat/long maths for a benefit nothing in G1–G9 needs.
**5C — metric only inside neighborhoods; the region is a node graph.** Loses G2's off-screen
geometry: no checkpoint has a position.

### C. Elevation and terrain

### 6. The elevation model

**6A — layered 2.5D (recommended).** A region carries a coarse heightfield; an authored
neighborhood carries fine heightfield tiles; a structure carries floors, each a layer with its own
height map and holes; bridges, tunnels, overpasses, decks and mezzanines are **layers**, not voxels.
`ground(x, y)` samples the finest covering tile; a room's `z` is its floor's offset above ground or
the ground itself outdoors; a stair or ramp is a portal with a `dz`. *Rests on:* G4; Mill Creek's
bridge, a station's decks and a ship's compartments all need two surfaces over one `(x, y)`.
*Changes:* the tile record and the resolver. **6B — one heightfield plus floors, no outdoor
layers.** Cheaper; cannot express a bridge over a road. **6C — voxels.** Rejected on cost and
authoring burden.

### 7. Resolutions and lattice sizes

**7A — authored per tile, sampled per domain (recommended).** Tiles carry their own resolution
(indoor ≈ 0.5–1 m, street ≈ 1–2 m, neighborhood outdoors ≈ 2–5 m, region heightfield ≈ 10–25 m);
a domain chooses one lattice from a ruleset-registered set at creation and composites the
substrate at that size; a domain never mixes lattices. *Rests on:* the 1 m bar fight and the 30 m
mech battle must share tiles (Q2). *Changes:* the compositing step and the cost bound (Q19).
**7B — one global resolution.** Either the bar is coarse or the valley is enormous. **7C — a fixed
resolution per ruleset.** The starter area alone needs two.

### 8. Background terrain for implicit space

**8A — deterministic procedural background (recommended).** Everything outside an authored tile is
generated on demand from the region's authored parameters (surface mix, relief amplitude, water
level) seeded by region id and coordinates, so an implicit corridor patch or a landing site is
reproducible without being stored; it is persisted only when something durable happens there;
authored tiles always override. *Rests on:* G6's landing sites and G2's off-screen space; storage
bounds. *Changes:* whether "materialize a patch" needs a write (no). **8B — flat, one class per
region.** Simplest; every landing site is the same field. **8C — procedural inside neighborhoods
too.** Rejected: authored areas are authored.

### 9. Destructibility scope

**9A — features carry damage; cells are immutable in v1 (recommended).** Walls, doors, bridges,
counters, gates, props are features with `Intact | Damaged | Destroyed` state that survives
regeneration; a crater or a breach is a feature laid over cells, not a cell edit. *Rests on:* doc 10
T2's boundary facts attach to objects; compositing stays a pure function of tile revisions plus
feature states. *Changes:* what the domain writes back at tick boundaries. **9B — editable cells
with a change log.** Needed later for floods and terraforming; not for a bar.

### 10. Surface and cover taxonomy

**10A — a small core set, ruleset-extensible (recommended).** Core registers surface classes
(clear, road, rough, woods, water, building, wall, …), cover classes (none, partial, full) and
passability by medium (land, air, water, space) as `CanonicalId`s; rulesets add classes and own
every cost and modifier. *Rests on:* one tile must serve WoD, Shadowrun and TacSim. *Changes:*
whether a pack authored under one ruleset loads under another (yes). **10B — ruleset-only
taxonomies.** Tiles stop being shareable.

### D. Rooms, exits, movement

### 11. Default timing of local movement

**11A — instantaneous by default; a portal's policy makes a specific exit timed (recommended).**
A door is a door; the brief marks the farm road or the long lane as timed (distance, mode, `dz`),
and those run on the same authority as journeys (Q12). *Rests on:* G1 puts the time between
neighborhoods, not in every doorway; MUSH pacing. *Changes:* nothing for existing rooms; the
portal record gains a policy. **11B — every exit timed by distance.** 300 m of street becomes a
30-second wait everywhere. **11C — timed outdoors, instant indoors.** A rule of thumb the brief
can express under 11A without making it law.

### 12. One travel system

**12A — one journey record grown from the carrier journey; `route.rs` retired (recommended).**
Travelers are a character on foot, a plain object, or a carrier (vehicle, ship) with its
interiors; a journey follows a **connection** (region-owned, two access points, timed legs with
checkpoints); local timed exits are one-leg journeys; the never-skip, catch-up-in-one-move,
grace-after-resume and `+intercept` behaviours of `route.rs` re-home in it. *Rests on:* G5 and the
three unrelated movement paths the tree has. *Changes:* `@route` goes; `@journey` becomes
`+travel`. **12B — keep two systems.** Interception and clocks diverge.

### 13. Rides without vehicles

**13A — both forms (recommended).** A **ride** is a character journey with a mode (`foot`, `car`,
`transit`) and no vehicle object — cheap, and what `+uber` is; an owned or named vehicle and every
ship is a **carrier** with interiors and occupants, which is what G6's transport is; a hired car
with an NPC driver (doc 07 §9) is a later layer over carriers. *Rests on:* object cost against the
"play aboard" experience. *Changes:* whether a taxi is an object (no) and a ship is (yes).
**13B — every ride is a carrier.** Every cab has a back seat room; heavy. **13C — rides for
everything but ships.** Owned cars deserve interiors too.

### 14. Travel time and clocks

**14A — journeys on `WorldIc` with per-mode real-time floors and caps (recommended).** A
connection's cost policy gives IC seconds; elapsed real time is IC divided by the campaign rate
(`[game].ic_ratio`, or `[tacsim.time].rate` where that campaign runs); a **minimum real dwell per
checkpoint** keeps interception possible when the rate compresses travel to nothing; a **maximum
real duration per mode** keeps a cross-city drive from consuming a real hour unless the campaign
wants it. The numbers are §5 I2. *Rests on:* doc 10 1A/6A and the two shipped rate mechanisms.
*Changes:* the connection record's fields. **14B — real time only.** TacSim's weeks become weeks.
**14C — instant with an IC jump.** Starfield fast travel; rejected by never-skip except on a
connection its author gave no checkpoints.

### 15. Checkpoints and interception

**15A — checkpoints are coordinates; a stake-out materializes them (recommended).** A checkpoint
is a point on a connection; a watcher who **stakes it out** (a verb) materializes a small patch
there from the background (Q8) and waits; a traveler reaching a staked-out checkpoint promotes an
encounter (a domain, or a narrative room for a conversation) before continuing; with no watcher
only the leg's hazard band applies, per ruleset policy. *Rests on:* C20A and the existing
hazard/interdiction bands; sprawl. *Changes:* `+intercept` generalizes from "same room" to "same
checkpoint". **15B — permanent tiny rooms at every checkpoint.** Sprawl by construction. **15C —
interception only through faction control records.** Loses the physical stake-out.

### 16. Destination knowledge and the travel verb

**16A — known destinations, server-chosen routes (recommended).** `+travel` lists public access
points plus places the character has visited or been told about (a plain "known places" set in
v1; the epistemic layer later); modes are filtered by what the character has; the server picks the
route by policy and shows alternatives only when they differ in mode or risk; secret access points
need a knowledge record. *Rests on:* doc 11 §4's experience and the belief model's absence.
*Changes:* how much knowledge v1 stores (a set per character). **16B — every public destination
always known.** Simpler; strangers know every bus stop. **16C — no verb, exits only.** Rejected by G1.

### E. Tactical domains

### 17. How a domain comes to exist and how big it is

**17A — scene-driven windows, staff-explicit too (recommended).** When a scene or a ruleset asks
for a map, the domain's extent is the union of the scene's anchor-room footprints plus a margin,
composited at the chosen lattice; it grows to a neighboring footprint when a participant leaves
the window (bounded); staff may create explicit extents. Pre-authored tiles are what the window
samples — the bar's map is authored (G2), the *domain* is the window over it. *Rests on:* doc 06
§11's scene ↔ domain binding and G2. *Changes:* compositing is a runtime step. **17B — one
pre-built domain per site, chosen at combat time.** No runtime compositing, but a fight that
spills into the street needs a second domain. **17C — whole-neighborhood domains, always coarse.**
The bar fight is coarse.

### 18. Audiences inside a domain

**18A — rooms stay the audience; the resolver decides which room (recommended).** `say`, `pose`
and `look` follow the room the pose resolves to; contacts and line of sight use the substrate;
rulesets may add radius hearing later. *Rests on:* doc 06 D1 and Q3. *Changes:* nothing for
players until a ruleset asks. **18B — domain-wide audiences.** The whole street hears the
barroom. **18C — metric radius for everything.** Q3B by another route.

### 19. Domain size policy

**19A — cap cells, coarsen rather than refuse (recommended).** A domain has a cell cap; a request
larger than it gets the next coarser lattice; `MAX_ACTIVE_DOMAINS` stays configurable. *Rests on:*
the K3 bundle limits and the "whole valley" case. **19B — refuse oversized requests.** The valley
fight becomes a staff ticket.

### F. Authoring, packs, the building agent

### 20. Pack granularity and references

**20A — one pack per neighborhood, one per region (recommended).** The region pack owns
localities, connections and corridors, background parameters and heightfield references; the
neighborhood pack owns sites, structures, rooms, portals, features, NPCs, lore, hooks and secrets;
a hydrated site is its own small pack registered as an addition to its neighborhood; cross-pack
references are `(pack, entry)` pinned through `requires` with an optional digest. *Rests on:* the
tacsim pack's `requires`; Git review units; digest stability. *Changes:* the bootstrap order (a
region before its neighborhoods) and the fix for 11a H3 (connections owned by the region).
**20B — one pack per region including all neighborhoods.** One digest moves on every edit.
**20C — one pack per site.** Hundreds of packs per city.

### 21. What an author gives the agent

**21A — prose, a partial pack, or a complete pack; the agent always produces a complete pack
(recommended).** A brief in prose is the common case; the agent drafts the pack, validates it,
lists what it had to invent or could not resolve as questions, and applies after the author's
go-ahead; a complete pack skips the drafting and still gets validation and questions. *Rests on:*
G7's flow as the owner stated it. *Changes:* the agent needs a pack schema as a tool, not only a
prose target. **21B — complete packs only.** Makes the author do the agent's job. **21C — prose
only.** Refuses the file the owner wants to write.

### 22. The agent's loop and budget

**22A — a bounded off-loop agent with a validator tool (recommended).** Tools: read the atlas and
the neighborhood's constraints, search existing sites, propose a pack, run the validator, revise;
at most N rounds (default 6) under one feature route (`worldgen_plan`), a per-build cost ceiling
and the actor's daily budget; the output is a pack plus a question list; nothing touches the
world until apply. *Rests on:* G7 and the budget/audit/kill-switch layer that already meters every
call. *Changes:* `mush-llm` gains a tool loop it does not have (only NPC turns loop today).
**22B — single-shot structured output with one repair retry.** Today's worldgen shape; cannot ask
questions or self-validate.

### 23. Do staff builds go live without review?

**23A — yes; the staffer is the reviewer (recommended).** A staff-initiated build applies live on
the staffer's go-ahead with an optional post-hoc ticket; a player-initiated hydration applies live
as `pending_review` under the neighborhood's policy (R3). *Rests on:* G7 and staff headcount (§5
I6). **23B — a second staffer reviews every build.** Safer, slower, and not what G7 says.

### 24. Where clarifying questions surface

**24A — a durable proposal record, answered in-game or on the web (recommended).** Questions
attach to the proposal, are paged to the author, answerable in any order and resumable across
restarts; the web UI reads the same record; a job ticket links to it for routing. *Rests on:* doc
11 §7's durable-conversation rules, kept. *Changes:* one record shape shared by staff builds and
player hydration. **24B — a job-ticket comment thread.** Untyped, and tickets are one-shot.
**24C — written back into the input file.** Fits "describe in a file"; can be an export of 24A
rather than a second loop.

### 25. Hydration defaults

**25A — any approved character, where policy allows; cap 2 outstanding (recommended).** A
neighborhood's policy says whether and what may be hydrated; any approved character may request;
at most two of a character's hydrations may be `pending_review` or generating at once (R3); the
reviewers are the neighborhood's builder list with staff fallback; nothing expires; duplicate
detection is advisory (kind, footprint, name similarity). *Rests on:* R3 needs a number. *Changes:*
the cap and the eligibility test. **25B — staff-approved requesters only.** Kills the "nobody
thought to build a bookstore" moment.

### G. The starter area and tests

### 26. Boot and test posture (the owner's rule, made concrete)

**26A — static pack, zero calls at boot, off in the testkit (recommended).** The starter area is
committed content (authored once, agent-assisted if wanted, reviewed like any pack); boot applies
it through the ordinary pack path with no LLM call; `[game].starter_area = true` by default for
real instances and `false` in `boot_world`'s minimal world; one suite applies it and asserts its
invariants; the agent path runs in tests only behind `MockProvider`. *Rests on:* G9. *Changes:* a
config key and a testkit default. **26B — generate at first boot when the LLM is on.** Rejected:
cost per instance and non-determinism.

### 27. The `Staff <S>` exit and the lot's texture

**27A — open by default, back rooms restricted; a film lot (recommended).** The exit is a tour, so
anyone may take it; the writers' room and props department are restricted; the texture is a film
lot, setting-neutral. **27B — staff-only from the start.** Then new operators never see the demo.
**27C — another texture** (owner supplies).

### H. What else moves onto the atlas

### 28. Themes, weather, control, property

**28A — theme and weather key on the atlas; zones keep locks and exits; faction regions and
housing properties gain atlas references (recommended).** Theme resolves room → site →
neighborhood → locality → region → `#0`; weather runs per region/neighborhood; `mush-faction::Region`
may reference a neighborhood or locality; `Property` may reference a site. *Rests on:* G5 makes
this cheap now and expensive later; three grouping mechanisms with no derivation is 11a M7.
*Changes:* `theme_chain` and the weather zone loop. **28B — only theme moves.** **28C — nothing
moves; the atlas is additive.** Coherent later, at migration cost.

### 29. Names and numbering

**29A (recommended).** Crate `mush-atlas` (records, tiles, resolver, travel) over `mush-content`
(packs, apply, latch); player verbs `+atlas`, `+travel`, `+stakeout`; staff `@atlas/*`; tiers region
/ locality / neighborhood / site / structure / room; the faction record is called a **control
region** in docs; doc 14 is the proposal, 14a this survey, 14b authoring and hydration UX, 14c
faction places; doc 11 is marked superseded by 14 when 14 lands. **29B — rename the geographic top
tier** (owner supplies the word). **29C — other** (owner supplies).

## 4. Defaults doc 14 will take unless vetoed

- Every content id is a pack-scoped `CanonicalId`; cross-pack references are `(pack, entry)`.
- Atlas records live in one `atlas` typed family under a registered owner and codec (09b); room
  membership is a record keyed by `ObjId`, never an attribute; tiles are immutable per revision.
- The pack digest discipline is `pack_digest`'s; the apply latch is the seed latch extended to
  create / update / retire with per-entry revisions and reference checks; export is the pack file;
  Git stays outside the server.
- Activation, review decisions and retirements are canon events; retirement is reviewed, never
  automatic.
- The canonical pose is `{ x, y, z, facing_deg, posture }`; the resolver falls back to the domain's
  presentation room; entry poses are free cells adjacent to the portal used.
- Cost bounds: tiles capped at 256×256 cells; domains capped in cells (Q19); `MAX_LEGS`,
  `MAX_ACTIVE_DOMAINS`, K3 bundle limits kept.
- Hydrated NPCs are created enabled at tier `minor` with the default per-NPC budget; a reviewer may
  park them; `secret` lore is the only enforced knowledge boundary (guarded is prompt framing).
- Presence/discovery (doc 11 §6) and faction places (§9) are out of doc 14's scope.
- The starter area's room budget is fifteen; its content is the review's worked shape (doc 11 §11
  R4) until the owner edits it.

## 5. Facts the proposal needs (not decisions)

| # | Needed | Why | Default if unanswered |
|---|---|---|---|
| I1 | Launch scale: neighborhoods, rooms per neighborhood, region extent — for the first WoD/Shadowrun game and for the corridor | tile counts, cell caps, pack counts | 6–12 neighborhoods of 8–20 rooms in one region ≤ 50 km square; the corridor as in cut 11 |
| I2 | Travel feel: real seconds for a cross-city drive and for a 500 m rural walk; IC:OOC per ruleset; minimum checkpoint dwell | Q14's floors and caps | WoD/SR rate 1:1 with a 3-minute cap per journey; TacSim per `[tacsim.time]`; 30 s dwell |
| I3 | Acceptable LLM cost per neighborhood build and per hydration | Q22's ceilings | $2 per neighborhood build, $0.50 per hydration, both configurable |
| I4 | Real map data (heightfields, streets) or fictional only | whether an import format is designed now | fictional; import format specified, not built |
| I5 | Expected concurrent players and simultaneous domains | caps and the compositing budget | ≤ 64 players, ≤ 8 domains |
| I6 | Reviewer capacity: staff and builder headcount; whether trusted players may review | R3's release rate, Q23 | staff plus designated builders only |
| I7 | Client priority for maps: web canvas, SSH text, or both first | the projection and renderer order | one projection; web canvas first, SSH text second |
| I8 | Whether TacSim ground combat must be hex (BattleTech feel) or may use squares | the second driver | hex available as a lattice; nothing hex-native |
| I9 | The starter area's names and voice (the lot's name, the receptionist's, the gamemaster's) | the pack | placeholders until supplied |

## 6. What doc 14 will contain

1. Model — the substrate (frames, layers, tiles, features), rooms and portals on it, connections,
   travelers and journeys, domains as windows, the resolver; invariants stated as testable
   properties.
2. Records — every record with fields, ids, bounds and its storage lane; the pack schema with the
   starter area as the worked example.
3. Behaviour — boot and apply; travel and interception; domain creation, compositing and
   write-back; elevation in narrative and mechanics; retirement.
4. The building agent — inputs, tools, validator checks, questions, budget, where it stops;
   staff and player paths; hydration states and quotas.
5. Cross-ruleset seams — what core owns, what rulesets register (lattices, classes, costs,
   modes), what the first street-level and TacSim drivers need.
6. Build order and acceptance scenarios, with the starter area as the fixture for each cut.
7. What it supersedes in docs 03, 06, 07, 10, 11 and the help topics, by section.

## 7. Answer sheet

| Q | Recommended | Owner's answer (2026-09-09) |
|---|---|---|
| 1 | 1A | **1A** |
| 2 | 2A | **2A** |
| 3 | 3A | **3A** |
| 4 | 4A | **4A, sharpened**: strike hexes everywhere; square grids for all maps (§8 S1) |
| 5 | 5A | **5A** |
| 6 | 6A | **6A** |
| 7 | 7A | **7A** |
| 8 | 8A | **8A** |
| 9 | 9A | **9A** |
| 10 | 10A | **10A, sharpened**: the engine defines the world; rulesets add definitions (§8 S2) |
| 11 | 11A | **11A** |
| 12 | 12A | **12A** |
| 13 | 13A | **13A** |
| 14 | 14A | **14A**; WoD runs IC at a multiple of real time, travel takes its IC time with a cap (§8 S7) |
| 15 | 15A | **15A** |
| 16 | 16A | **16A** |
| 17 | 17A | **17A** |
| 18 | 18A | **18A** |
| 19 | 19A | **19A** |
| 20 | 20A | **20A** |
| 21 | 21A | **21A** |
| 22 | 22A | **22A**; $5 hard cap, $2 target (§8 S6) |
| 23 | 23A | **23A** |
| 24 | 24A | **24A** |
| 25 | 25A | **25A, sharpened**: cap 2, staff-settable per character, strikes to zero, good conduct earns more (§8 S3) |
| 26 | 26A | **26A, sharpened**: ship built, and ship the brief so it can be reconceptualized (§8 S4) |
| 27 | 27A | **27A** |
| 28 | 28A | **28A** |
| 29 | 29A | **29A** |
| I1–I9 | defaults in §5 | **defaults**, except I3 ($5 hard / $2 target), I7 (SSH first, §8 S5), I8 (no hexes) |

## 8. The owner's sharpenings (2026-09-09, binding)

- **S1 — No hexes.** "Remove any references to hexes because they're difficult to display in
  text/SSH; prefer an x-y grid for all maps." Applies to every map, driver and document; doc 14
  D-14-4 records it and §8 there lists the superseded hex references in docs 04, 06 and 10.
- **S2 — The engine defines the world.** Terrain types and the movement impacts on bipeds,
  wheeled and tracked movers are engine-level concepts; rulesets provide definitions such as
  weapons that use the engine's line-of-sight, projectile and laser/energy rules, and may extend
  the vocabularies. Doc 14 §5.
- **S3 — Caps are staff-settable with strikes.** The default cap of two outstanding reviews is
  per character and staff-settable; a player who behaves earns a higher cap; one who generates
  trash earns strikes and is set to zero. Doc 14 §4.5.
- **S4 — Built and reconceptualizable.** Ship a built starter area with no LLM involvement, and
  ship its brief, so a staffer can re-run the authoring workflow from the Staff Lobby brief with
  different answers. Doc 14 §3.7.
- **S5 — SSH first for maps.** Anything SSH can display the web can display; the reverse is not
  true; the SSH client must never be unable to show something critical. Doc 14 §6.
- **S6 — Cost.** $5 is an appropriate hard limit for a large neighborhood; $2 a reasonable
  average target. Doc 14 §4.1.
- **S7 — WoD time.** WoD campaigns run IC time at a multiple of real time so vampires are not
  stuck inside all day; travel takes whatever its IC time is, with a cap. Doc 14 §3.3.
