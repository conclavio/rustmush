# 14 — The integrated spatial/room system: implementation plan

**Draft 0.1 — 2026-09-09.** The build plan for [doc 14](14-spatial-room-system.md) (Draft 0.1,
decisions D-14-1..32 binding), in the shape of [10b](10b-e7-implementation-plan.md) and
[11-h](11-h-implementation-plan.md): seven cuts, each independently CI-green and adversarially
verified before the next builds on it, every record and verb named, every gate listed. Grounded
in the tree at `0bc5e92` (code unchanged since) by path and symbol; no line numbers. Nothing here
is built.

Doc 14's §7 lists six cuts; this plan splits its cut 2 into a travel half and an adoption half,
adds the interface NPCs (D-14-32) as their own cut, and keeps the order. There is **no
migration**: no atlas data exists, the starter area is the first content, and every record is
new — so no compatibility shims, no dual-read, no backfill. The one thing every cut inherits is
the rule that boot spends nothing (P9).

---

## 1. Scope — one line per cut

| Cut | One line | Doc 14 |
|---|---|---|
| **S1** | `mush-content` (packs, digest, latch with update/retire) and `mush-atlas` (records, rasterizer, `+atlas` text views, `@atlas/pack`); the starter packs applied at first boot | §2, §3.1, §3.2, §2.6 |
| **S2** | One travel system: connections, journeys for characters, objects and carriers, `+travel`, `+stakeout`, holds, timed portals, the tour bus; `route.rs` retired | §3.3 |
| **S2b** | The atlas adopts theme, weather, control regions and property (D-14-28) | §0 D-14-28 |
| **S3** | The building agent and hydration: the tool loop, the validator as a tool, proposal records, staff and player paths, caps and strikes, reconceptualize-from-brief | §4 |
| **S4** | Windows and the street driver: tiles, features, floors, composite, metric pose, resolver, write-back, `+map`, the `atlas.map` panel in text and on the web | §3.4, §3.5, §6, §5.4 |
| **S5** | Engine physics: the locomotion table, line of sight, fire and beam traces, hearing; `mush-propagation` on the metric | §5 |
| **S6** | Interface NPCs: the interface-ability provider, asker-scoped authority, the starter cast's abilities | §4.6 |
| **S7** | The TacSim ground driver (10 m cells, indirect fire), heightfield import, the hex vocabulary removed | §5.4, §3.5, §8 |

### 1.1 Explicitly out

Presence and discovery (doc 11 §6 → 14c), faction places (doc 11 §9 → 14d), the in-game survey
UX beyond what §4's proposal record needs, the epistemic layer (`known_places` is a plain set),
NPC routines, orbital and system node-graph *content* (the records exist from S1; the corridor's
packs stay `ruleset-tacsim`'s until S7 folds them), and any rewrite of `@gen`'s single-room
description path (it keeps working; S3 adds the planner beside it).

## 2. What the plan consumes as landed, by symbol

- **Packs and identity.** `crates/ruleset-tacsim/src/pack.rs`: `PackManifest { id, version,
  kind, digest, requires }`, `PackKind`, `PackRequirement { id, version, digest }`, `load_pack`,
  `check_requires`, `pack_digest` (canonical JSON), `RulesPack::{manifest, entries, entry,
  canonical_value, computed_digest}`, `PackEntry::{Commodity, Recipe, Route, Location, Facility,
  Deposit, Formation}`; `seed.rs`: `SeedPlan`, `PlannedObject`, `apply` (objects and latch in one
  `WriteBatch`, never re-run); `record.rs`: `SeedApplication`; `store.rs`: `TacsimStore`,
  `put_seed`, the latch key `tacsim:seed:{pack}@{version}`. `crates/mush-core/src/mechanics.rs`:
  `CanonicalId` (`[a-z0-9_.]`, `CANONICAL_ID_MAX_BYTES` = 64), `DefinitionDigest`;
  `action.rs`: `IdempotencyKeyText`; `resource.rs`: `ReservationRecord`, `ReservationBook`.
- **Placement, domains, journeys.** `crates/mush-sim/src/data.rs`: `SpatialState`,
  `SpatialParent`, `PlacementRecord`, `DomainRecord`, `DomainClock::{Passive, Pulse}`,
  `SpaceAnchor`, `InteriorSpaceRecord`; `graph.rs`: `project`, `verify_projections`;
  `transition.rs`: `Transition`, `Plan`, `plan`/`apply`; `domain.rs`: `MAX_ACTIVE_DOMAINS`,
  `PromotionTrigger`, `DomainActivation`; `route.rs`: `RouteDef`, `LegDef`, `MAX_LEGS`,
  `RouteCatalog`, `Journey`, `Leg`, `RESUME_GRACE_S`, the sweep; `journey.rs`: `CarrierJourney`,
  `advance_due`, `reanchor`, `MAX_CARRIER_SUBTREE`; `commands.rs`: `cmd_journey`,
  `journey_plan|depart|divert|show|list`, `arrival_parent`, `build_route`, `route_create`,
  `pose_from`, `PASSIVE_DRIVER`; `lib.rs`: the `MoveAttempt`/`CarryAttempt` gates.
- **Tactics and propagation.** `crates/mush-tactics/src/mobility.rs`: `MobilityVocabulary::base`
  (`ground/air/water/space`; `legged/wheeled/tracked/hover/rotor/fixed_wing/displacement/thrust`;
  tags), `MobilityMode { id, mediums, locomotion, tags, rules, nominal_speed_cms, transition_s }`,
  `MOBILITY_ID_MAX`; `lib.rs`: `TacticsStub`, `toy_map`, `HexLock`;
  `crates/mush-propagation/src/context.rs`: `Geometry { near, far, range, occluded }`, `Medium`,
  `ADJACENT_RANGE`, `MAX_CONTEXT_RECORDS`.
- **Core seams.** `crates/mush-core/src/module.rs`: `Registrar::{command, command_gate,
  behavior, tick, hook, panel_feed, query_handler, job_queue, migrations, state_family,
  scene_tick_start}`, `PanelFeedHandle::{keyframe_to, delta_to}`, `HookPoint::MoveAttempt`,
  `Ctx::set_map_blob`/`map_blob` (to be retired); `cmd.rs`: `CommandCompatibility`;
  `crates/mush-contract-tests/legacy-handlers.toml` and its test (every new verb listed in the
  same commit with `read-only | legacy-post-image | authoritative`, a `legacy-post-image` entry
  naming its family); `clock.rs`: `ClockRef`, `ClockStamp`, `World::world_ic_now`;
  `obj.rs`: `LIMBO`, `Obj`; `crates/mush-canon/src/data.rs`: `CanonDraft`.
- **LLM, NPCs, worldgen, jobs.** `crates/mush-llm/src/service.rs`: `LlmFeature`, `CallCtx`,
  `KillSwitch`; `provider.rs`: `BudgetScope`, `LlmError`, `ToolDef`/`ToolChoice` (`types.rs`);
  `mock.rs`: `MockProvider`; `crates/mush-npc/src/snapshot.rs`: `AgentGrounding::{profile,
  invoke_ability, projected_ability}`; `lib.rs`: `NpcAction`, `MAX_ROUND_TRIPS`, `MAX_ACTIONS`;
  `turn.rs`: the bounded tool loop; `knowledge.rs`: `Secrecy`, `LorePool`; `profile.rs`:
  `AgentProfile`; `crates/mush-ruleset/src/lib.rs`: `Ruleset::{agent_profile, invoke_ability}`,
  `AgentCapabilityProfile`; `crates/mush-worldgen/src/theme.rs`: `THEME_ATTR`, `theme_chain`,
  `resolve_theme`; `lib.rs`: `WorldgenConfig`; `crates/mush-jobs/src/api.rs`: `open_job`,
  `register_payload_handler`, `PayloadHandlerFn`; `data.rs`: `Job`, `JobPayload`;
  `crates/mush-market/src/projection.rs`: `projection(catalog, market, snapshot, hidden_orgs,
  now)`; `crates/mush-weather/src/lib.rs`: the per-zone cell (`mod:weather:cell:{zone}`).
- **Server, config, tests.** `crates/mush-server/src/modules.rs`: `server_modules`,
  `server_modules_with` (composition order); `persist.rs`: the first-boot minimal world (Limbo
  `#0`, God `#1`) written through before serving; `crates/mush-config/src/config.rs`: `[game]`
  (`ic_ratio`, `ic_epoch`), `[onboarding]` (`start_room`, `start_room_unapproved`), `[worldgen]`,
  `[llm.routes.*]`, `[llm.budgets]`, `[tacsim.time]`; `crates/mush-testkit/src/lib.rs`:
  `boot_world`, `TestWorld::{new, with_modules, spawn_player, run, set_now, fire_due,
  drain_write_batch, assert_saw}`; `.github/workflows/ci.yml`: `test` (fmt, clippy, build,
  test, TS bindings drift), `features`, `kernel-contracts` (panic surface, per-crate lints,
  legacy allowlist, migrations), `redis-faults`; `panic_budget/<crate>.txt` per crate;
  `docs/help/*.md`.

## 3. Cut structure

### 3.1 Cut S1 — `mush-content`, `mush-atlas`, the starter packs

**Crates.** `mush-content` (`{core, proto, serde, serde_json}`): the pack layer, neutral.
`mush-atlas` (`{core, content, proto, serde, serde_json, smol_str, tracing}`): the records, the
rasterizer, the views. Both get `panic_budget/<crate>.txt` entries and the per-crate restriction
lints from day one.

**Records — `mush-content`.** `PackManifest`, `PackRequirement`, `load_pack`, `check_requires`,
`pack_digest` and the canonical-JSON discipline move here **unchanged in bytes**: a tacsim pack's
digest computed by `mush-content` equals the one `ruleset-tacsim` computes today (a test pins
three fixture digests). `PackKind` gains `Region | Neighborhood | Site`. Entries become
`{ kind: CanonicalId, def: serde_json::Value }` decoded by an **entry codec registry**: each owner
(`ruleset-tacsim`, `mush-atlas`) registers `EntryCodec { kind, decode, validate }` at
composition; an unknown kind fails the load naming the entry. The latch generalizes
`SeedApplication` to `Applied { pack, version, digest, applied_at, placed: BTreeMap<CanonicalId,
ObjId>, revisions: BTreeMap<CanonicalId, u64> }` at `mod:content:applied:<pack>`, and
`apply::{plan, apply}` produce and execute `Plan { create, update, retire, findings }` in one
`WriteBatch` with the latch — `seed::apply`'s invariant ("objects and the latch or neither")
kept verbatim. **`ruleset-tacsim` keeps its own `SeedApplication` and `@tacsim/seed` this cut**
(★ D-P-1): folding its latch onto the shared one is S7's, when the corridor's locations become
atlas entries.

**Records — `mush-atlas`.** Doc 14 §2.1–2.3 verbatim: `RegionRec`, `LocalityRec`,
`NeighborhoodRec`, `SiteRec`, `StructureRec`, `Floor`, `TileRec` (declared now, populated by
the rasterizer at coarse resolution), `FeatureRec`, `RoomGeo`, `PortalRec`, `ConnectionRec`
(declared now; journeys are S2), `AccessPoint`. One typed family `atlas` registered through
`reg.state_family` with strict wire codecs and deterministic keys (`mod:atlas:<kind>:<id>`); room
membership at `mod:atlas:roomgeo:<obj>`; tiles at `mod:atlas:tile:<id>:<rev>`, immutable. The
`Cells` encoding: per cell `class u8, elev_cm i16, cover u8, room u16` = 6 bytes, run-length on
`class` rows, base64 in the document; 256×256 cells ≤ 320 KiB before RLE (cut 1 measures the
real size against the store's document limit and records it in §7).

**The rasterizer.** `atlas::raster::{footprints, portals, walls, floors}` — pure functions from
a decoded neighborhood pack to `RoomGeo`, `PortalRec`, `FeatureRec` and coarse `TileRec`s per
doc 14 §3.2; the background recipe `atlas::background(region, surface, x, y)` — a seeded hash of
`(region.id, surface, x/res, y/res)` into class and elevation from the recipe's parameters (P4).

**Commands** (listed in `legacy-handlers.toml` in the same commit): `+atlas` · `+atlas <place>` ·
`+atlas/region` (**read-only**); `@atlas/pack list|show <id>|plan <file|id>|apply <plan-digest>|
retire <id>` (**legacy-post-image**, family `atlas.records`; staff, `SIM_ADMIN`/Royalty). `apply`
requires the digest `plan` printed, so an edited pack re-plans.

**Wiring.** `mush_content::ContentModule::new()` composed in `server_modules_with` **before**
`mush-sim` (it owns no world state, only the latch family); `mush_atlas::AtlasModule::new()`
immediately after `mush-sim`. Boot: after `persist.rs` seeds `#0`/`#1`, `build_engine` applies
the starter packs when `[game].starter_area` (new `GameCfg.starter_area: bool`, default `true`)
through the ordinary `apply`, in `requires` order, with the LLM layer untouched; the testkit's
`boot_world` sets it `false`; `TestWorld::with_starter_area()` is the one way a test gets the lot.
The packs are `include_str!` fixtures under `crates/mush-atlas/content/` — `region.limbo.json`,
`neighborhood.ooc_area.json`, `neighborhood.staff_lobby.json` — with their `brief` blocks; a test
pins their digests so an unreviewed edit fails CI.

**Config.** `[game].starter_area = true`; `[atlas]` new: `tile_max_cells = 65536`,
`nearest_room_cells = 8`.

**Tests (~44).** *Identity:* three tacsim fixture digests unchanged through `mush-content`; a
reordered entry map does not change a digest; a hyphenated id is refused naming the entry;
an unknown entry kind is refused naming the entry. *Latch:* plan → apply → plan is empty;
apply twice creates nothing (P8); a retire of an occupied room, a room with an inbound exit
from outside the pack, a scene anchor, a titled property is refused naming the reference; a
crash between objects and latch is impossible by construction (one batch — the tacsim S-lens
test ported). *Geometry:* Lantern-Ward-style overlapping site footprints are refused (11a M9);
room footprints partition per surface (P3); every exit row yields a portal on the shared edge or
an explicit-distance journey portal. *Background:* two calls agree; nothing is written (P4).
*Boot:* first boot with the starter area on makes zero `MockProvider` calls (P9); `boot_world`'s
world has exactly `#0` and `#1`; `with_starter_area()` has fourteen rooms plus `#0`, the two
exits on `#0`, and the cast at their posts. *Views:* `+atlas` renders the lot in text within
80 columns; a room the viewer cannot see is absent, not blanked.

**Verify lenses.** **P** (pack identity — can two byte streams make one digest, or one two?),
**I** (idempotence — can any interleaving create a second object or lose the latch?), **G**
(geometry — can any pack pass validation with an overlap or an orphan?), **B** (boot — can any
configuration make boot call a provider or make a test world grow?).

**What this cut does not do.** No journeys, no windows, no agent, no theme or weather changes.

### 3.2 Cut S2 — one travel system

**Records.** `ConnectionRec`, `Checkpoint`, `AccessPoint` populated from region packs (the
starter's `connection.lot_tour`); `JourneyRec` (doc 14 §2.4) **replacing** `CarrierJourney` —
same cursor, `advance_due`, `reanchor`, `applied_revision` and anomaly semantics, plus
`Traveler::{Character, Object, Carrier}` and `state: UnderWay | Held | Arrived | Aborted`; the
journey document moves to `mod:atlas:journey:<id>` under the `atlas` family; `PortalRec.time`
honoured; `known_places` at `mod:atlas:known:<char>` (a bounded set, `MAX_KNOWN_PLACES`).
`route.rs`'s `Journey`, `Leg`, the sweep and `@route` are **deleted**; `RouteDef`/`LegDef` move
into `ConnectionRec.legs` (bytes unchanged: `LegDef`'s wire form is kept so `ruleset-tacsim`'s
route entries decode as before).

**Behaviour.** `atlas::travel::{depart, advance_due, hold, release, cancel}` on one `Coarse`
tick (`atlas.travel`, 1 s), IC-clock stamped; a character or object traveler projects into
`in_transit_room` between checkpoints and into the checkpoint's patch room while `Held`; a
carrier moves its subtree exactly as `journey.rs` does today. **Timed portals:** `mush-atlas`
registers a `MoveAttempt` hook that, for a portal with `time: Timed`, starts the one-leg journey
and vetoes the instantaneous move with the departure narration. **Stake-outs:**
`+stakeout <checkpoint>` materializes the patch (a room created from the background, retired
when the last watcher leaves and nothing durable happened) and registers the watch; a traveler
reaching a watched checkpoint is `Held` and both sides are told; `+intercept` is "hold the
traveler at my checkpoint or in my room". **Clocks:** `elapsed_real = ic / rate` with
`[atlas].dwell_floor_s` per checkpoint and `[atlas].real_cap_s.<mode>` per mode; a rate change
re-anchors every journey (`reanchor`, kept).

**Commands.** `+travel` · `+travel <dest>[/<mode>]` · `+travel/status` · `+travel/cancel` ·
`+stakeout <checkpoint>` · `+stakeout/end` · `+intercept <traveler>` (all
**legacy-post-image**, family `atlas.journey`; `+travel` bare and `/status` **read-only**);
`@atlas/journey list|show|hold|release|abort` (staff); `@route` and `@journey` removed from the
allowlist in the same commit.

**Wiring.** `AtlasModule` gains the tick and the hook; `mush-sim`'s `journey.rs` and the
`@journey` handlers are deleted; `TacsimRouteCatalog` (`crates/mush-server/src/sim_routes.rs`)
implements the connection lookup for the corridor's packs until S7.

**Tests (~46).** *Never-skip:* a traveler visits every checkpoint in order at or after its due
time; a catch-up after a `set_now` jump lands in one move with no narration; a watcher at any
checkpoint sees the traveler (P6). *Clocks:* a rate of 4 arrives sooner but never inside the
dwell floor; the cap holds a four-hour IC drive to `real_cap_s`; a rate change mid-journey
re-anchors and never strands (P7). *Travelers:* a character on foot, a plain object and the
tour bus with three players and a mounted prop aboard traverse the same connection with one
location each throughout (P1); a passenger boarding mid-hold rides the rest. *Holds:* cancel and
disconnect leave the traveler at its last checkpoint; a destroyed traveler ends the journey;
restart resumes from the cursor. *Portals:* the farm-road pattern as a timed portal; a short
door stays instantaneous. *Knowledge:* `+travel` lists only known places; `+intercept` reveals
no destination.

**Verify lenses.** **N** (never-skip under every clock path), **C** (clock arithmetic across
generations), **S** (subtree integrity under carrier motion), **K** (knowledge — can any
rendering leak a destination or a route?).

**What this cut does not do.** No hired-car dispatch (doc 07 §9 later), no epistemic knowledge,
no windows: a hold is a narrative room until S4 lets a scene open a window on it.

### 3.3 Cut S2b — the atlas adopts theme, weather, control regions and property

**Records.** `theme: Option<Theme>` on `RegionRec`, `LocalityRec`, `NeighborhoodRec`, `SiteRec`
(the `Theme` struct from `crates/mush-worldgen/src/theme.rs`, moved to `mush-atlas`);
`resolve_theme(room)` walks room → site → neighborhood → locality → region → `#0`, same merge
rules (nearest scalar wins, lists unioned); `worldgen.theme` on ZMOs is no longer read.
Weather cells keyed by neighborhood or region id (`mod:weather:cell:<atlas id>`); a room's
weather is its neighborhood's, else its region's; zones keep locks and zone exits only.
`mush-faction::Region` gains `atlas: Option<CanonicalId>`; `mush-housing::Property` gains
`site: Option<CanonicalId>`; neither is required.

**Commands.** `@atlas/theme <place>=<field>:<value>` · `@atlas/theme/show [<place>]` ·
`@atlas/theme/exemplar <place>=<room>` (replacing `@theme`); `@weather <place>` where `<place>`
is an atlas id or a room; `@region/atlas <control-region>=<place>`; `@property/site <property>=
<site>`. All **legacy-post-image** with their families.

**Tests (~20).** Theme resolves through the atlas chain and ignores the ZMO attribute; a room in
no neighborhood falls to its region then `#0`; weather transitions broadcast to a neighborhood's
outdoor rooms; a control region with an atlas reference renders its footprint in `+territory`;
a property's site link is refused when the rooms lie outside the site's footprint.

**What this cut does not do.** No rendering of control on the map (S4's projection may add it).

### 3.4 Cut S3 — the building agent and hydration

**`mush-llm`.** A generic bounded **tool loop** (`agent::run(ctx, tools, rounds, budget)`)
extracted from `mush-npc`'s `turn.rs` so both use one implementation: typed `ToolDef`s, one
repair retry, `MAX_ROUND_TRIPS`-style bounds, per-call settle, and a **per-run cost ceiling**
(`RunBudget { cap_usd, target_usd }`) enforced at pre-check like every other budget.
`LlmFeature::WorldgenPlan`, route `worldgen_plan` (`[llm.routes.worldgen_plan]`, a strong
model, `effort = "high"`), a per-feature cap.

**`mush-atlas::validate`.** The deterministic validator of doc 14 §4.3 as a library function
over a decoded pack plus the world (`Findings { code, entry, severity, message, fix_hint }`);
`@atlas/pack plan` and the agent's `pack.validate` tool call the same function.

**`mush-worldgen` — the planner.** `ProposalRec { id, author, scope, source: Prose | Pack,
revision, pack: Option<Pack>, findings, questions, answers, feedback, status, cost_usd,
rounds }` at `mod:worldgen:proposal:<id>` (typed family `worldgen.proposal`); tools
`atlas.read`, `atlas.search_sites`, `pack.propose`, `pack.revise`, `pack.validate`, `ask`; the
run is off-loop and re-enters as `WorldCommand::ProposalUpdated { id, revision }` (treaty ruling
3); stale results are dropped by revision. **Reconceptualize:** `@atlas/propose --from <pack>`
seeds the run with the pack's `brief` and its current entries; the resulting plan diffs against
the latch.

**Hydration.** Reservations through `ReservationBook` (C8) for the character's outstanding slot,
the neighborhood's capacity and the footprint; the request id an `IdempotencyKeyText` derived
from `(character, neighborhood, kind, footprint)` so duplicates converge; activation as a `Site`
addition pack with `origin: Hydrated`, `review: PendingReview`; a `BUILD` job queue
(`JobQueueSpec { name: "BUILD", min_role: Builder }`) with payload handler `build.review`
carrying `{ request, pack_digest, neighborhood, schema_version }`; canon events
`atlas.site.{activated, approved, changes_requested, struck, retired}`. Caps and strikes per
doc 14 §4.5: `hydration_cap` and `strikes` on the character (`mod:atlas:standing:<char>`).

**Commands.** `@atlas/propose <prose | file | --from <pack>>` · `@atlas/cap <char>=<n>` ·
`@atlas/strike <site>=<reason>` · `@atlas/standing <char>` (staff); `+build/show [<id>]` ·
`+build/answer <id>/<q>=<a>` · `+build/feedback <id>=<text>` · `+build/go <id>` ·
`+build/withdraw <id>` · `+build/status` · `+build/place <kind> [<where>]=<prose>` (players);
`+job/approve|deny` on the `BUILD` queue drive review. Categories: the `+build` writers
**legacy-post-image** (family `worldgen.proposal`); `+build/go` on a staff proposal rides
`@atlas/pack apply`'s path; `+build/show` and `/status` **read-only**.

**Config.** `[worldgen]`: `plan_cost_cap_usd = 5.0`, `plan_cost_target_usd = 2.0`,
`plan_max_rounds = 6`, `hydration_cap = 2`, `hydration_strikes_to_zero = 3`;
`[llm.routes.worldgen_plan]`; `[llm.budgets.per_feature.worldgen_plan]`.

**Tests (~52), all through `MockProvider` with scripted tool calls.** *Loop:* a prose brief
becomes a validating pack in ≤ 6 rounds; a pack that never validates leaves a resumable
proposal with findings; the cost cap stops the run mid-round and nothing is applied; the target
is told to the model and never enforced. *Authority:* no tool mutates the world (a probe counts
`WriteBatch` writes during a run: zero); `+build/go` by a non-author is refused; a staff go
applies live; a player go applies `pending_review` and opens one ticket. *Quota:* two
outstanding requests refuse a third; approval releases a slot; three strikes set the cap to
zero and a fourth request is refused naming the standing; `@atlas/cap` raises it. *Convergence:*
two concurrent requests for one implied park yield one request id, one charge, one site.
*Reconceptualize:* `--from neighborhood.staff_lobby` with different answers yields a plan whose
diff keeps the bus stop's access point. *Boot:* still zero provider calls (P9). *Secrets:* a
pack with `secret` text in `persona` is refused by the validator.

**Verify lenses.** **A** (authority — can any tool result reach the world without a go?), **$**
(cost — can any path exceed the cap or bill the wrong actor?), **Q** (quota — can any
interleaving exceed the cap or dodge a strike?), **I** (idempotent requests).

**What this cut does not do.** No interface NPCs (S6); no fine terrain: sites activate with the
rasterizer's coarse tiles and `tactical_ready = false`.

### 3.5 Cut S4 — windows, the street driver, rendering

**Records.** `WindowRec` on `DomainRecord` (`window: Option<WindowRec>`); `Pose` replaces the
opaque `Direct { pose: RulesValue }` payload (`posture` stays a `RulesValue`); the composite
document at `mod:tactics:map:<domain-id>` (owner `atlas`, immutable per `terrain_revision`);
`FeatureRec.state` write-back through the `atlas` family at tick boundaries; fine tiles from the
rasterizer at `100 cm` for indoor floors and `200 cm` for the lot's outdoors.

**Behaviour.** `+map/open` from a scene facilitator (or `ScenesModule` on an engagement start),
`@atlas/window <bbox>` for staff: extent = union of anchor footprints plus `[atlas].window_margin_m`,
`cell_cm` from a ruleset-registered set (`street = 100`), coarsened past
`[atlas].window_max_cells`; composite off-loop → `WorldCommand::WindowComposited`; placement
on open per doc 14 §3.4; the resolver (`resolve_room`) drives `Obj.location` and every
`say`/`look` audience (D-14-18); growth on edge crossing; write-back of feature states with the
domain tick. `Ctx::set_map_blob`/`map_blob` and `toy_map` are deleted; `+btest` uses a real
window over the lot. A generic `+move <cell | dir> [<n>]` (scene-class `Turn`, gated by the
scene like `+roll`) moves one's own pose with cost 1 per cell until S5 supplies the table.

**Rendering.** `atlas.map` panel feed (`reg.panel_feed("atlas.map")`), keyframe/delta with
`{ window, revision, tick, origin, cell_cm, rows: [glyph runs], legend, units: [{ id, cell,
glyph }] }` **already filtered for the viewer**; the **text renderer first**: `mush-ansi` gains the
treaty's panel text-fallback registrar (`reg.panel_text(feed, renderer)`), `+map` prints the
frame in line mode, the TUI gains a map screen; the web gets `web/src/lib/stores/map.svelte.ts`
and a canvas view from the same frames; the TS bindings drift check covers the new frame type.

**Commands.** `+map` · `+map/open` · `+map/close` · `+map/floor <n>` · `+move …` (**legacy-post-
image**, families `atlas.window` / `sim.placement`; `+map` bare **read-only**); `@atlas/window
list|open|close`.

**Tests (~54).** *Purity:* the same pins and feature states composite byte-identically twice
and after a restart; closing and reopening changes no cell (P5). *Placement:* entry poses are
free cells adjacent to the portal used; a full room refuses; leaving resolves to the containing
footprint, then the nearest within 8 cells, then the presentation room (P1). *Elevation:* the
Production Office's pose has `z = 3.6`; the stair portal has `dz = 3.6` (P2). *Growth:* a
participant stepping off the edge grows the window into the Backlot's tiles within the cap.
*Write-back:* a `Destroyed` counter survives a description regeneration and appears in the next
composite. *Text:* golden frames for the Coffee Shop and the Backlot at 80×24 (P10); a viewer
receives no cell or unit outside their visibility (V). *Web:* the bindings compile; a keyframe
followed by a delta renders the same grid as the text frame.

**Verify lenses.** **P5** (purity), **T** (text sufficiency — does any frame carry a fact the
text drops?), **V** (visibility — can any frame leak a hidden unit or a cell the viewer cannot
see?), **R** (resolver — can any pose be in two rooms or none?).

### 3.6 Cut S5 — engine physics

**`mush-atlas::physics`.** The engine vocabularies of doc 14 §5.1 (surface classes, cover,
opacity; locomotion classes taken over from `MobilityVocabulary::base`, `biped` added; media);
the locomotion table as data (`content/locomotion.json`, digest-pinned) with `passable`, `cost`
and slope limits; `move_cost`, `path` (bounded A*, `[atlas].path_max_cells`), `line_of_sight`,
`direct_fire_path`, `indirect_fire_path`, `beam_path`, `hearing`, `resolve_room` — every query
constant-work per cell visited and oracle-free (a miss and a hit read the same records).
`mush-propagation` computes `Geometry { range, occluded }` from `line_of_sight` and metric
distance when both ends share a window, else keeps its graph answer; `Medium` maps onto the
media. `+move` uses `move_cost`; rulesets read the traces.

**Tests (~40).** Every locomotion row (biped on road 1.0, tracked in deep water impassable,
hover over shallow water passable, slope limits); LOS blocked by a wall, not by a window
feature, blocked by a rise in elevation, partial through light woods; a direct trace stops at
the first blocker and reports the target's cover; an indirect trace clears a wall below its
apex; a beam attenuates through partial opacity; hearing attenuates by distance and features;
propagation's range over the lot equals the metric and its occlusion equals LOS.

**Verify lenses.** **D** (determinism — identical inputs, identical traces, across restarts),
**O** (oracle-freedom — timing and reads independent of hit or miss), **X** (extension — a
ruleset-added class or locomotion cannot change a core row).

### 3.7 Cut S6 — interface NPCs (D-14-32)

**`mush-npc`.** `InterfaceAbilityProvider` (`abilities() -> Vec<ToolDef>`, `invoke(w, out,
AbilityContext { npc, asker, principal, powers }, name, input)`) registered beside the ruleset
grounding; `AgentProfile.interfaces: Vec<CanonicalId>` names which providers an NPC fronts;
`AgentCapabilityProfile` merges the interface manifest into the tool set; `invoke_ability`
threads the **asker** (the speaking player of the stimulus) so every read is filtered as that
character's own command would be, and every write is a proposal write the asker owns.
Providers: `mush-atlas` (`atlas.describe`, `who.nearby` — presence-safe: co-present and
advertised only), `mush-weather` (`weather.report`, `weather.forecast`), `mush-market`
(`market.quote`, `market.list` over `projection`), `mush-worldgen` (`build.propose`,
`build.status`, `build.answer`, charged to the asker's actor budget under `worldgen_plan`),
`mush-ruleset` help (`chargen.explain`, `rules.lookup`, `help.topic`), `mush-atlas` travel
(`travel.next`, `travel.status`), `mush-jobs` (`request.file`, `request.status`, the asker's own).
Fallback lines name the command. Audit rows carry `(npc, asker, ability)`.

**Content.** The starter packs' NPC entries gain `interfaces`; Susie, Karen Smith, Regina, Cady
Heron, Janis Ian, Phil and (if the bench is accepted) Duvall and Norbury front the abilities of
doc 14 §2.6; Aaron drives and fronts nothing.

**Commands.** `@agent/set <npc>/interfaces=<a; b>` (staff). No new player verbs: talking is
the interface.

**Tests (~36).** *Authority:* a hidden control region is invisible through Gretchen to a player
and visible through her to a wizard, from the same lore; a market's hidden seller is absent from
Cady's answer; Ron cannot show another player's ticket. *Mutation-freedom:* a probe counts world
writes during any interface turn: zero except the proposal record. *Cost:* a `build.propose`
turn settles against the asker's actor budget and the `worldgen_plan` feature, not the NPC's; a
weather turn settles against the NPC's. *Fallback:* with the LLM off each NPC names its
command. *Audit:* every ability row names the asker.

**Verify lenses.** **A** (authority), **M** (mutation-freedom), **$** (attribution).

### 3.8 Cut S7 — the TacSim ground driver, imports, and the last hexes

**Scope.** `cell_cm = 1000` windows for TacSim ground engagements over the same tiles;
`legged`, `tracked`, `wheeled`, `hover`, `rotor` costs from the table; indirect fire wired to
the pulse driver's intents (E7 cut 8); `heightfield` pack entries (16-bit little-endian grid or
text grid) imported to ground tiles; the corridor's `location` and `route` entries become atlas
`SiteRec`/`ConnectionRec` entries in an orbital/system region, and `ruleset-tacsim`'s seed
folds onto `mush-content`'s latch (★ D-P-1's second half); `TacticsStub`'s `toy_map`, `HexLock`,
the `tactics.hex` driver name and every "hex" in `docs/design/04`, `06`, `10` and the help topics
are removed or reworded (D-14-4). **The campaign ephemeris (doc 10 T17):** `body` pack entries
`{ orbit_au, period_days, phase_deg_at_epoch }`, `[tacsim.time].ephemeris = { epoch, rate = 0 }`;
at load, every connection between two bodies' places gets its distance computed from the phases
at the epoch and pinned; `route_duration` gains the drive class's distance law (`torch`: time ∝
√distance; `coast`: ∝ distance) in place of the unapplied `alignment_band`, which is deleted.

**Tests (~26).** A mech engagement on imported relief composites at 1000 cm from the same tiles
a street fight used at 100 cm; an indirect shot clears a ridge; the corridor's packs load
through `mush-content` with unchanged digests; `grep -ri hex crates docs` finds only the
history in this plan and the review. *Ephemeris:* the pinned Mars–Ceres and Earth–Mars
distances match the real geometry at the epoch within 2%; with `rate = 0` a route's duration is
identical across any IC span (T17); with a test-only nonzero rate every pair sharing a body
changes together and no pair contradicts another; a torch route's duration scales with the
square root of its pinned distance.

**Verify lenses.** **D**, **P** (pack identity across the fold), **H** (no hex left), **E** (the sky — can any path make a distance change while `rate = 0`, or two routes disagree about one pair?).

## 4. Build discipline

- **One cut, one branch, one verify fold.** Each cut lands green on `test`, `features`,
  `kernel-contracts` and `redis-faults`, then gets an independent adversarial verify over its
  lenses whose findings are folded before the next cut starts — 10b's pattern, unchanged.
- **Lints and panics.** The per-crate restriction lints (WS01-S12) and the panic budget files
  apply to `mush-content` and `mush-atlas` from their first commit; no `unwrap`, `expect`,
  panicking index or `?` after the first irreversible mutation in any tick or apply path
  (`journey.rs`'s rule).
- **Lanes.** Every new verb is listed in `legacy-handlers.toml` in the commit that adds it, with
  its category and family; mutating atlas state rides the `atlas` typed family through
  `WriteBatch`; nothing writes an attribute for a fact the family owns.
- **Boot spends nothing.** Every cut keeps the P9 test; the `MockProvider` call counter during
  boot is asserted zero in `mush-server`'s boot tests and in `TestWorld::with_starter_area()`.
- **Text first.** A panel feed lands with its text renderer in the same cut; the web view
  follows in the same cut or is explicitly deferred in the cut's deviations.
- **Help topics.** `docs/help/atlas.md` (S1), `travel.md` (S2, replacing `route.md`), `build.md`
  (S3), `map.md` (S4); `worldgen.md`, `spatial.md`, `npc.md`, `tui.md`, `weather.md`, `faction.md`
  revised in the cut that changes them. Help stays truthful about shipped behaviour at every
  commit.
- **Docs.** Each cut's deviations and gates are recorded under its section here (the 10b
  ledger); doc 14's §7 table points at this plan.

## 5. Coordination

- **E7 cut 8** (the pulse driver) is S7's dependency for indirect fire and the ground driver;
  S4's windows do not need it (a street fight is scene-turn gated).
- **`mush-propagation`** is consumed by S5; its `Geometry` contract does not change shape.
- **H1** (the store-read seam) is how S4 pages tile documents off-loop for compositing.
- **14c presence** will read `who.nearby`'s eligibility set from S6; nothing in S1–S7 emits
  a presence feed.

## 6. Decisions taken by this plan (defaults; ★ marks an owner call)

| # | Decision |
|---|---|
| D-P-1 ★ | `ruleset-tacsim` keeps its own seed latch through S1–S6 and folds onto `mush-content`'s in S7 |
| D-P-2 | `mush-content` before `mush-sim`, `mush-atlas` after it, in `server_modules_with` |
| D-P-3 | Starter packs are `include_str!` fixtures in `mush-atlas` with digest-pinning tests |
| D-P-4 | The journey document moves under the `atlas` family; `route.rs` is deleted in S2, not deprecated |
| D-P-5 | Theme and weather move in S2b, not S1, so S1 has no behavioural change outside new verbs |
| D-P-6 | The tool loop lives in `mush-llm` and `mush-npc` becomes its first caller (one loop, two users) |
| D-P-7 | `+move` is a generic atlas verb gated by the scene turn, not a ruleset verb |
| D-P-8 | The composite stays at `mod:tactics:map:<domain-id>` (doc 06 §9.1's key) with owner `atlas` |
| D-P-9 | Feature-state write-back rides the domain tick's commit, never a second timer |
| D-P-10 ★ | Aaron Samuels drives the bus from S2 (decided); Norbury and Duvall ship in S6 if the owner nods, otherwise the packs carry the eight cast members |

## 7. Constants

| Constant | Value | Where |
|---|---|---|
| `CANONICAL_ID_MAX_BYTES` | 64 | existing |
| `MAX_LEGS` | 32 | existing, kept |
| `MAX_ACTIVE_DOMAINS` | 8 | existing, kept |
| `tile_max_cells` | 65 536 (256×256) | `[atlas]` |
| `window_max_cells` | 65 536 | `[atlas]` |
| `window_margin_m` | 10 | `[atlas]` |
| `nearest_room_cells` | 8 | `[atlas]` |
| `path_max_cells` | 4 096 | `[atlas]` |
| `dwell_floor_s` | 30 | `[atlas]` |
| `real_cap_s` (`foot`, `car`, `transit`, `bus`) | 180 | `[atlas]` |
| `MAX_KNOWN_PLACES` | 512 | `mush-atlas` |
| `plan_cost_cap_usd` / `plan_cost_target_usd` | 5.0 / 2.0 | `[worldgen]` |
| `plan_max_rounds` | 6 | `[worldgen]` |
| `hydration_cap` / `hydration_strikes_to_zero` | 2 / 3 | `[worldgen]` |
| street `cell_cm` / TacSim ground `cell_cm` | 100 / 1000 | ruleset registration |
| starter area rooms | 14 + Limbo | packs |

## 8. Open questions (defaults chosen)

- **Cell encoding size** against the store's document limit: measured in S1; if 256×256 at 6
  bytes exceeds it after RLE, tiles drop to 128×128 and `tile_max_cells` follows.
- **Where the composite's off-loop work runs**: the world task's existing off-loop pattern for
  LLM work (snapshot in, typed command out) is the default; a dedicated compositor thread only
  if S4 measures the composite over a few milliseconds on the lot.
- **Scene-turn gating of `+move` in freeform scenes**: default free (no gate) outside a
  turn-based scene, so a walk across a window is not a fight.
- ★ **D-P-1** and ★ **D-P-10** above.

## 9. What this plan hands back

- **Doc 14 §8's supersessions become deletions** as each cut lands: `route.rs` (S2), the ZMO
  theme read (S2b), `set_map_blob` and `toy_map` (S4), the tactics stub's hex vocabulary (S7).
- **14c (presence) and 14d (faction places)** start after S6, on `who.nearby`'s eligibility set
  and on `SiteRec.property` respectively.
- **The hired car** (doc 07 §9 `MobilityJob`) layers on S2's journeys and S6's interface
  abilities: a dispatcher NPC fronting `travel.request`.

## 10. Changelog

- 0.1 (2026-09-09): first draft, from doc 14 Draft 0.1; S7 gains the campaign ephemeris (doc 10 T17) the same day.
