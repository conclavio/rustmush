# 14 — The integrated spatial/room system: implementation plan

**Draft 0.8 — 2026-09-10 (S1 through S6 landed).** The build plan for [doc 14](14-spatial-room-system.md) (Draft 0.1,
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

**Ledger (landed 2026-09-09).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo build --workspace --all-targets`, `cargo test --workspace`,
the restriction-lint table (both crates at zero on all six), the panic budget (no file: zero), the
legacy-handler allowlist (`+atlas` read-only; `@atlas` legacy-post-image, family `atlas.records`),
the feature and CI-gated inventories. Verify fold over P/I/G/B: see the findings folded below.
Deviations from the text above, each with its reason:

- **One loader, not a moved one.** `mush-content` is a new pack layer that digests the *raw
  canonical JSON* of a pack (manifest without its digest, entries as authored, the brief), so a
  pack's identity is owner-independent: two servers with different decoders compute one digest.
  `ruleset-tacsim`'s loader stays where it is, byte for byte, and its typed-entry digests are not
  what `mush-content` computes — the "three tacsim fixture digests unchanged" test is therefore
  not written; the fold of tacsim onto `mush-content` is S7's (D-P-1), and that is when the
  tacsim digests get re-pinned. Entries decode through an `EntryApplier` per kind (`kinds`,
  `phase`, `create`, `update`, `retire_check`, `retire`, `finish`, `finish_retire`), registered
  at composition by `mush-atlas` (thirteen place kinds) and `mush-npc` (`npc`, `lore`); an
  unknown kind is refused at plan and at apply naming the entry, and a hyphenated id at parse.
- **The latch is per entry and honest.** `Applied { pack, version, digest, applied_at, entries:
  id → (kind, entry digest, object), retired, anomaly }` at `mod:content:applied:<pack>`. An
  apply that stops part way — an owner refused an entry, the world refused an object — is
  latched as far as it got with the anomaly, never rolled back (tacsim's seed rule, kept for
  update and retire); the next apply plans from that latch. A retired pack keeps its latch
  marked `retired`, so a first boot never resurrects what staff removed.
- **Objects and the latch commit together at boot.** `boot_modules` folds the world's dirty
  objects into the same boot `WriteBatch` as the module ops (it wrote only ops before S1, and
  before S1 no module made objects at boot). Without that, a crash between the latch write and
  the world's first flush would leave a latch describing a lot that is not there.
- **Atlas records ride the compatibility lane, not `reg.state_family`.** Bounded whole documents
  with strict `deny_unknown_fields` wires re-validated at boot, at `mod:atlas:<kind>:<pack>:
  <entry>`, `roomgeo:<obj>` and `tile:<pack>:<entry>`; a document that fails its wire is dropped
  loudly with a `Del` fixup (`mush-party`'s rule — records are derived from packs and a pack
  re-applies). The state-family machinery is for K-owned authoritative families with
  coordinator hydration, and a tile takes part in no authority or prepared mechanic. The
  allowlist entry names `atlas.records` as the family the records move to when S4's windows put
  tiles under the K3 bundle limits.
- **Composition order.** `AtlasModule` composes right after `mush-sim` as planned;
  `ContentModule` composes **last**, after the TacSim layer, not before `mush-sim`: its boot pass
  applies packs whose entries are made by other modules' appliers, each writing its own RAM slot
  (the NPC host's lore pools, the atlas's records), and `on_world_loaded` runs in list order, so
  every owner must have hydrated first or its own boot would overwrite the pass. Boot applies a
  shipped pack only when it has **no latch at all**; a newer shipped version over an older latch
  is logged and left to `@atlas/pack plan`, so a restart never rewrites a lot under its players.
- **`[game].starter_area` gates the boot packs, not a testkit flag.** The testkit composes no
  content module, so `boot_world` is Limbo and God by construction; there is no
  `TestWorld::with_starter_area()`, because a `mush-testkit → mush-atlas` edge would put a
  dev-dependency cycle under every crate that tests through the testkit. A test that wants the
  lot composes `NpcModule`, `AtlasModule` and `ContentModule::with_boot_packs(starter::packs())`
  (`crates/mush-atlas/tests/lot.rs`); the production roster gets it from
  `modules::content_module(config.game.starter_area)`.
- **P9 is asserted as "no effect deferred", not as a mock call count.** `build_engine` builds
  its own `LlmService`, so no `MockProvider` can be injected into a production boot; the lot
  test drives `Engine::modules_loaded` directly and asserts the boot deferred no `Effect` at
  all, which is the only way a call could leave the world task, and `boot_modules` keeps its
  `debug_assert!(fx.is_empty())`.
- **`@atlas/pack plan <file>` is served by `[atlas].pack_paths`.** Packs are read host-side at
  boot onto a shelf (tacsim's discipline: a bad file fails the boot naming it), never from a
  path inside the world task; `plan`, `apply` and `retire` name shelf packs. `apply` takes the
  sixteen-hex fingerprint `plan` printed (a digest over the pack digest and every step), and
  refuses when the pack or the world moved since.
- **The two neighborhood packs do not import in either order.** The Staff Lobby's bus exits
  name the OOC Area's bus room, so `neighborhood.staff_lobby` requires `neighborhood.ooc_area`
  and applies after it; out of order is refused naming the requirement. The bus is a room
  (`room.bus`) standing at both its stops until S2 gives it a journey; that makes fourteen lot
  rooms, the bus interior, and Limbo.
- **Tiles** are produced by the appliers' `finish` pass once a pack's entries are all in place:
  one ground tile per neighborhood at its resolution (100 cm), one per structure floor, and the
  region's coarse ground (1000 cm), each coarsened — never refused — until it fits
  `tile_max_cells` and 512 KiB encoded. Measured (`tile_encoding_sizes_are_measured`): a flat
  256×256 tile encodes to 28 bytes, a full 8 m relief lattice to 85 KiB, and only a grid where
  no cell equals its neighbour reaches 683 KiB, which the coarsening rule keeps out of the
  store. The background's relief and mix are drawn on an 8 m lattice for exactly that reason.
  §8's open question is closed: 256×256 stands.
- **Interface abilities are not in the NPC entries.** The cast carries profiles, fallbacks and
  lore; `interfaces` arrive with S6, as a pack revision the latch applies in place.

Verify fold (P/I/G/B), findings folded before landing: **P** — the digest is over the raw
canonical value, so a re-keyed entry map or re-ordered object digests identically and a
hand-edited digest fails `DigestMismatch` naming both. **I** — an entry id is unique only within
its pack, and the first retire implementation looked records up by entry id alone, so retiring
the Staff Lobby's `neighborhood` entry removed the OOC Area's record: `retire` and `retire_check`
now take the pack id and address records by `(pack, entry)`. A destroyed object stays a `GOING`
husk until the world task sweeps, so counts in the tests exclude husks. **G** — the `region`
entry's `#[serde(flatten)]` kind did not survive `deny_unknown_fields` (serde refuses the
flattened field), so the region pack applied nothing at first; the def now carries `kind` and
`body` as plain fields. **B** — covered by the boot-batch deviation above.

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

**Ledger (landed 2026-09-09).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
-- -D warnings` and `--all-targets` on the crates touched, `cargo test` on every dependent, the
restriction-lint table (`mush-atlas` still at zero on all six), the legacy-handler allowlist
(`+travel`, `+stakeout`, `+intercept` legacy-post-image, family `atlas.journey`; `@atlas` gains
`/journey`; `@route` retired, the baseline pin lowered to 155), the digest pins (the three starter
packs re-pinned at version 2). Verify fold over N/C/S/K: see the findings folded below.
Deviations from the text above, each with its reason:

- **The carrier journey is layered, not replaced** (D-P-4, amended by the owner at S2).
  `journey.rs`, `@journey` and `TacsimRouteCatalog` stay for placement-graph carriers until
  S7's TacSim fold; `mush-sim`'s movement gate keeps only its carrier arm. What is deleted is
  `route.rs`'s runtime — `Journey`, `Leg`, `RouteState`, the sweep, `@route`, `+intercept` and
  the boot hydration — plus `tests/route.rs` and cut 6's "legacy `@route` is untouched" test;
  `RouteDef`, `LegDef`, `DriveProfile`, `route_duration` and the catalog seam are what remain.
- **`JourneyRec` is the atlas's own record**, not a moved `CarrierJourney`: `Traveler`
  (character, object, carrier), `Way` (a connection or a timed portal), `Heading`, the resolved
  `nodes` (origin room, checkpoints, destination room) and `legs_ic` **fixed at departure**, the
  two WorldIc stamps, the cursor, `Held { by, since } | UnderWay | Arrived | Aborted`, a real-time
  `grace_until`, and `applied_revision` = the way's revision — a connection or portal re-applied
  under a journey ends it at its last node (the "failed validation" arm of doc 14 §3.3) instead
  of cut 6's placement heal, which has no meaning for a room projection. Terminal records are
  retired on the spot; a live one is `mod:atlas:journey:<id>`, strict wire, re-validated at boot.
- **The cursor, the catch-up and the re-anchor are cut 6's** (one boundary per slice, filter
  before take, relabel never re-base), and **release re-bases** the departure so every leg still
  to run runs whole from the release — a shift by the hold's length, `route.rs`'s rule, restarted
  nothing when the hold began late in a catch-up (found by test N3 and fixed).
- **Clocks** are `plan_legs`: real = IC / rate; the per-mode `real_cap_s` scales every leg down
  proportionally, the `dwell_floor_s` then lifts every leg **after the first** (the dwell at the
  checkpoint it leaves) to the floor, and rounding that carries the whole over the cap is taken
  back from the longest legs. A paused world (rate 0) plans as rate 1 and nothing is due.
- **A read-only hook cannot start a journey.** `MoveAttempt` vetoes a timed portal with the
  departure narration and **queues** the departure (`Arc<Mutex<Vec<_>>>` shared with the tick);
  the next tick, at most a second later, departs it if the actor is still at the exit. The same
  hook gates a traveler under way or held (no exit, no teleport) and opens a vehicle's doors only
  at the stop it stands at; `LookRendered` adds the traveler's position line in a transit room;
  `Perceive` hides two travelers at different points of one road from each other (P1: the room
  is a projection of progress, not a place).
- **A timed portal is any exit whose rooms do not touch** (S1's `distance_m` rule already made
  it `Timed { foot }`); `portal.timed = [modes]` overrides the modes. The leg is the distance at
  the mode's speed plus two seconds a metre of climb, then the floor and cap.
- **The transit room and the patch are runtime atlas rooms**, one per way
  (`mod:atlas:transit:<key>`) and one per materialized checkpoint (`mod:atlas:patch:<key>`, with
  a held vehicle's doors), made on first use from the region's background and retired when
  nobody watches, nobody is held and nothing stands there. **Anyone standing in a patch is a
  watcher** — a passenger who stepped off a held bus holds the road as surely as the person who
  typed `+stakeout` — so a hold made by a watch lifts when nobody is left, whoever made it, and
  `+stakeout/end` works for them too, walking back to where they came from or to the road's
  first end. A watch is not persisted: at boot the watchers are whoever stands in a patch.
- **The tour bus is a room that travels**: `connection.service { vehicle, dwell_s, driver }`
  (entry references resolved through the content latch at run time, since the vehicle's pack
  applies after the region's) runs the vehicle end to end on its own clock, dwelling `dwell_s`
  real seconds at each stop; passengers keep their one location, the doors open where the bus
  stands or onto the patch it is held at, and the driver's PA lines are scripted from the pack
  (D-P-10's "Aaron drives" is this, not the S6 agent loop). `+intercept` on a vehicle standing
  at the interceptor's stop holds it there (RAM only; `+intercept/release` lets it go).
- **The last leg of a connection names the connection itself** (its far end), so a connection's
  legs cover the whole corridor and a reverse journey walks them backwards; a `LegDef` still
  decodes byte-for-byte. The starter packs move to **version 2** for this and for the service
  (with the bus's text); a world that applied version 1 keeps its lot, drops its v1 connection
  record loudly at boot (it no longer validates) and gets the tour back with `@atlas/pack apply`.
- **`+travel` from the road.** A traveler whose journey ended past a checkpoint (cancel,
  disconnect, a closed way) stands in the transit room; `+travel` there offers the road's two
  ends from the node they stopped at (RAM; after a restart, from the near end). With no mode
  named, a road you can walk is chosen before a ride, then the quicker, then by name.
- **Two verbs beyond the list**: `+intercept/release <traveler>` (the counterpart the text
  lacked) and `@atlas/journey send <object>=<place>[/<mode>]` (how a plain object gets on the
  road: the old `@route`'s one legitimate use, for staff).
- **Knowledge grows by visiting only.** `known:<char>` is written by the `Moved` observer for a
  player entering an authored room; "being told" is not in S2. A hidden connection is offered
  only to someone who knows its far neighborhood; staff see everything.
- **What is RAM and forgotten by a restart**: watches (rebuilt from who stands in a patch),
  intercepts, a vehicle held at its stop, and a stranded traveler's node. A disconnect ends a
  character's own journey where it is (doc 14 §3.3); a passenger's disconnect ends nothing.
- **Tests (22, not ~46)**: `tests/travel.rs` covers every bullet of the *Tests* paragraph above
  in eighteen end-to-end tests over a test valley (two neighborhoods, a foot road with two
  checkpoints, a hidden road, a four-hour drive, a farm road) and the starter lot (the bus), plus
  four unit tests on the planner and the wire; several bullets share one test. The count is
  lower than planned because the lot and the valley exercise one engine, not two.

**Verify fold (N/C/S/K).** Six findings, all fixed before landing: **N** a release that shifted
the departure by the hold's length re-ran nothing after a late hold (now re-bases); **N** the
watcher on their way out still counted as a watcher, so their own `+stakeout/end` never released
what they held (now excluded), and a hold whose watcher was someone else never lifted when that
person left (the sweep now lifts any watch-hold with nobody left); **C** rounding each capped leg
up carried a bus passage two seconds over `real_cap_s` (now taken back from the longest legs);
**S** `@atlas/journey send` took the first road by map order — the four-hour drive — when the
foot road was meant (one chooser for `+travel` and `send`); **K** an exit's `<HINT>` leaked into
the departure prose (stripped). Left as noted: the intercept and the door-hold are RAM.

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

**Ledger (landed 2026-09-09).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
-- -D warnings` and `--all-targets` on the crates touched, `cargo test` on every dependent,
the restriction-lint table (`mush-atlas` still at zero on all six), the legacy-handler
allowlist (`@theme` retired with its verb; `@weather`, `@region` and `@property` moved from
baseline to legacy-post-image with families `weather.cell`, `faction.region` and
`housing.property`; the baseline pin lowered to 151; `@atlas` gains `/theme`). Deviations
from the text above, each with its reason:

- **`Theme` moved to `mush-atlas::theme`** (with `DescLength`, the merge rules and
  `resolve_theme`); `mush-worldgen` re-exports them and its `theme.rs`, `@theme`,
  `theme_on` and `theme_chain` are gone. `mush-worldgen`, `mush-weather`, `mush-faction` and
  `mush-housing` now depend on `mush-atlas` — four acyclic edges, since the atlas depends
  on `{core, content, proto}` only.
- **The chain and the world theme.** `resolve_theme(room)` walks the room's site,
  neighborhood, locality and region and then the **world theme**, a new atlas document
  `mod:atlas:theme:world` edited as `@atlas/theme world=…` (`#0` and `default` are
  aliases). The plan's chain ended at `#0`: the `worldgen.theme` attribute on `#0` is still
  read as the world default when no atlas document exists, so a world themed before the
  atlas keeps its default; the attribute on a zone or a room is no longer read (D-14-28),
  and no room-level override exists — a room off the map resolves to the world theme.
- **A theme on the records** (`theme: Option<Theme>`, `serde(default)`, validated with
  byte and count caps) can be authored in a pack (`theme` on a region, locality,
  neighborhood or site definition) or set by staff; a re-apply that authors none keeps the
  staff-set theme, since the record is otherwise rebuilt from the definition.
- **A place** (`mush_atlas::place::PlaceRef`, `find_place`) is the one lookup the theme,
  the weather, the faction and the housing modules share: by name (exact, then a unique
  prefix or substring), by pack id for a region or a neighborhood, `<pack>:<entry>` for a
  locality or a site, `world`.
- **Weather cells key on the pack id** of a neighborhood or a region
  (`mod:weather:cell:<id>`, `WeatherCell.place`); a room's cell is its neighborhood's, else
  its region's, so a region-wide climate covers every neighborhood that has none of its
  own. A pre-atlas zone-keyed cell (`cell:<dbref>`) is dropped loudly at boot with its
  document deleted, and the `ObjDestroyed` cleanup is gone with it (places are not objects).
  `@weather <place>` takes a name, a pack id, `here` or a room (its neighborhood); a locality
  or a site is refused as a weather key. The panel payload keeps its `zone` field name for
  the web client, now carrying the place's name. The flavor top-up reads the theme of the
  first place using a climate through the atlas.
- **`Region.atlas` is a string id**, not a `CanonicalId`: a control region may stand on a
  locality (`<pack>:<entry>`) as well as a neighborhood or a region. `+territory` appends
  `[map: <kind> <name>, <w> m × <h> m at (<x>, <y>)]` — the extent, since the plan reserves
  drawing control on the map for S4. Its `/zone` coverage stays.
- **`Property.site` is a `RecordKey`**, since a site is a pack entry, not a pack. The link
  is refused naming the first room that is on no map or whose footprint's corners fall
  outside the site's polygon (a room whose geometry names the site passes outright), and
  cleared with an empty right-hand side; `/show` prints it. Title, tenancy and the access
  predicate are untouched.
- **Boot order does not matter to any of it**: the four modules read the atlas's slot at run
  time and validate nothing against it at their own boot, so a weather cell for a place the
  atlas has not applied yet (or has retired) simply has no rooms until it does.
- **What migrated.** The weather crate's three suites, `mush-worldgen`'s theme tests, and
  the `mushd` weather, worldgen, flavor and persistence end-to-end tests now drive places —
  the starter lot's OOC Area in the daemon tests, small fixture packs elsewhere — instead of
  zone master objects; the help pages for worldgen, weather, faction and the atlas say what
  keys on the atlas.
- **Tests (fourteen new or rewritten against the plan's ~20)**: the theme chain through the
  atlas ignoring the attribute, the world default and the off-map fall-through, the pack
  and staff theme on a record; weather by id, name and room, the neighborhood-else-region
  rule, flags, the glance, ambient suppression, the query seam, panels, the round trip and
  the legacy-cell drop; a control region's footprint in `+territory`; a property's site link
  and its refusals.

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

**Ledger (landed 2026-09-10).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test` on `mush-llm`, `mush-npc`, `mush-atlas`,
`mush-content`, `mush-jobs`, `mush-config`, `mush-worldgen`, `mush-canon`, the dependents
(`mush-weather`, `mush-faction`, `mush-housing`, `mush-sim`), the contract tests (the panic
budget — `mush-atlas` and `mush-worldgen` still at zero sites, no budget raised — the
restriction-lint table, the legacy-handler allowlist with `+build` added as legacy-post-image,
family `worldgen.proposal`, and `@atlas` gaining `/propose`, `/cap`, `/strike`, `/standing`; the
baseline pin unchanged at 151), `mush-server` and `mushd` whole. Deviations from the text
above, each with its reason:

- **The tool loop is `mush_llm::agent::run`** over a `ToolHost` trait (`call`, `on_text`,
  `finished`): the loop owns the rounds, the echo of only the answered tool-use blocks, one
  repair, and the per-run cost cap; the host owns what a tool means. `mush-npc`'s dialogue
  turn is a `DialogueHost` on it (its suites unchanged). The cap is a pre-check inside the
  loop over the run's *reported* spend (`usage.cost_usd`), not a ledger scope: the ledger
  meters days, a run is minutes, and a provider that reports no price spends nothing against
  the cap, which is the honest answer. The host is `Send` so the drivers' spawned runs are.
- **The validator** (`mush_atlas::validate`) takes a serializable `ValidateContext` captured on
  the world loop for a scope (the world, a region, a neighborhood), so the agent's off-loop
  `pack.validate` runs the same function over the world as it stood when the run began and
  `@atlas/pack plan` runs it over the world as it stands; `+build/go` re-validates against the
  present before applying. Findings are typed with the plan's five fields; an error refuses
  the apply, a warning prints through, an info is a note. Rule 7 marks unknown travel modes,
  missing service vehicles and NPC abilities staff-run as infos (no pack names a ruleset
  mechanic yet, so no registry is consulted); rule 8's "never touches live-state fields" is
  structural (the wires deny unknown fields) and its "retire has no live references" is
  occupancy and exits from outside the pack. The site-kind taxonomy is a warning, never a
  refusal (R1). `@atlas/pack apply` refuses a blocking finding before it reads the fingerprint.
  The starter packs validate clean, as a regression test; the geometry suite's base map
  gained a way in from Limbo, since every public room is now expected reached.
- **Reservations are the request records**, not a `ReservationBook` (C8): a request at
  `mod:atlas:request:<id>`, keyed by an idempotency key over (character, neighborhood, kind,
  footprint hash), holds the character's slot, the neighborhood's unit and its footprint
  while it is outstanding, and every check re-derives the three from the outstanding set.
  Reason: they are two small counts and one footprint, the atlas takes no authoritative
  action yet, and the K family (`reg.state_family` under the composition authority, an
  `ActionTxn` per hold) belongs with the first one; the allowlist stanza says so. A footprint
  left unsaid is the first unclaimed twenty-metre square. Standing is
  `mod:atlas:standing:<char>`; `@atlas/cap` sets the cap **and clears the strikes** (the reset
  D-14-25 names).
- **The re-entry is the existing seam.** There is no `WorldCommand::ProposalUpdated`: the run
  rides `Effect::ModuleAsync` in the worldgen envelope (`WorldgenReq::Plan`) and re-enters as
  `WorldCommand::ModuleAsyncDone` carrying `GenOutcome::Plan`; the proposal's `revision`
  (bumped by every answer, feedback and run start) is what drops a stale outcome.
- **`@atlas/propose` hands off through `AtlasHooks`**, a function the worldgen module
  registers at its boot, since the atlas has no LLM edge. `+job/deny` reaches the atlas through
  a new status-observer seam on the jobs module (`register_status_observer`), beside the
  payload handler; the `BUILD` queue (min role Builder) is the atlas's, the `build.review`
  handler and the `atlas.build` observer register when jobs is composed. A staff `+build/go`
  applies live and files a payload-less BUILD ticket for the record.
- **Site packs** resolve their neighborhood through `requires` (the applier learned it), and
  the run fixes the manifest — id, kind, version, requires — so the model gives entries only:
  `site.hyd_<char>_<hash>` for a request, `site.p<id>_<slug>` / `neighborhood.p<id>_<slug>` /
  `region.p<id>_<slug>` for a staff brief by scope, the pack's own id at the next version for
  `--from`. `atlas.read` and `atlas.search_sites` answer from the captured context.
- **`file` is not an in-game source**: packs arrive on the shelf (`[atlas].pack_paths`) and
  `--from` reconceptualizes them; the prose form takes `<place>=<prose>`.
- **Config**: `[worldgen]` carries the five knobs as written; the two hydration knobs reach
  the atlas through `atlas_config(cfg, worldgen)` (`hydration_strikes_to_zero = 0` is a boot
  error). `[llm.routes.worldgen_plan]` and `[llm.budgets.per_feature].worldgen_plan` are in the
  example and the deploy template.
- **Tests (28 new against the plan's ~52 scenarios; each named scenario covered)**: the loop's
  echo, repair, round bound, cap-before-a-call, finished host and refusal (6); the validator's
  clean starter packs and block, schema, geometry, topology, policy-secrets-theme, diff
  safety, and the plan/apply reading (7); hydration's slot-unit-ground-convergence, activation
  and review both ways, three strikes and the reset, allowance and a closed policy, the
  planner hand-off, a reboot (6); the planner's brief-to-pack-to-go with zero calls at boot,
  the resumable proposal with a stale drop, the cost cap, the target told and not enforced, a
  staff go with a record ticket and a stranger refused, questions and answers,
  reconceptualize keeping the way in, a secret refused, a reboot (9).

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

**Ledger (landed 2026-09-10).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test` on `mush-sim` (with `testkit`), `mush-atlas`,
`mush-scenes`, `mush-tactics`, `mush-core`, `mush-proto` (with `ts`, regenerating the bindings),
`mush-ansi`, `mush-net-ssh`, `mush-gateway`, the dependents, the contract tests (the panic
budget — `mush-atlas` still at zero sites — the restriction-lint table, the legacy-handler
allowlist with `+map` added as legacy-post-image, family `atlas.window`, and `+move` as
legacy-post-image, family `sim.placement`; `@atlas` gaining `/window` and `/feature`; the
baseline pin unchanged at 151), `mush-server` and `mushd` whole, and the web client's
`svelte-check` and vitest. Deviations from the text above, each with its reason:

- **The composite re-enters through the atlas's own `Effect::ModuleAsync` completion**, not a
  new `WorldCommand::WindowComposited` arm: `mush-core`'s engine doc forbids a core arm per
  feature (the generic `ModuleAsyncDone` carries an opaque module payload), so the world task
  gained an `atlas` routing arm that runs the pure compositor on a blocking thread and
  re-enters with the typed outcome; a stale revision is dropped on-loop. §8's default stands:
  no compositor thread of its own.
- **The window document is `mod:atlas:window:<domain>`** — the owner's namespace, so it returns
  to the atlas at boot — and it is the whole window (frame, anchors, rooms, pins, the composite),
  immutable per revision: a re-composite is a new revision, never an edit. `terrain_revision` is
  the per-tile `revision` the composite pins. A window whose composite never landed (the process
  died between open and re-entry) is closed at boot rather than re-kicked, since boot defers no
  effect.
- **`WindowRec` sits on `mush-sim`'s `DomainRecord`** in integer centimetres and display-form
  keys (the record is `Eq`); **`Pose`** replaces the opaque payload with the posture as its only
  rules value, and a cut-1 document still decodes; **the resolver is a slot** (`RoomResolver`)
  the atlas fills at boot, because the atlas depends on the simulation and not the reverse.
  A domain with a window projects through it and its participants walk its exits: the
  placement gate stands down for windowed domains, and the atlas's `Moved` observer places,
  re-poses or releases on the far side.
- **No separate 200 cm outdoor tiles**: the windows sample the tiles S1 made (the lot's
  neighborhoods at 100 cm, the region's ground at 1000 cm); the window's cell size, coarsened
  through the registered sizes past `window_max_cells`, does the coarsening (D-14-19).
- **The text fallback is shape-driven**: a panel frame whose `schema.shape` is `grid` renders in
  `mush-ansi` from its data alone, because the renderer sits across the IPC boundary from the
  registry, where a `Registrar::panel_text` closure could not reach. The frame's schema is the
  I9 convention with one more shape.
- **The TUI's map is the unit screen's declared grid pane** (the geometry projection it
  awaited), not a tenth screen: the nine digits are taken, and the scene screen keeps its
  layout (its goldens unchanged); line mode prints the frame in the scrollback wherever `+map`
  is typed.
- **`+map` is one legacy-post-image command** (its switches write; a switch is not a command),
  family `atlas.window`; `+move` rides `sim.placement`. `+move` outside a turn-based scene is
  free (§8's default). A room's access stands in for the exit's lock on a walk until S5's
  table; the walk crosses floors at a doorway at the same height; a step off the edge grows the
  window and stops the walk until the new composite lands.
- **Write-back is immediate**: street windows have no pulse clock, so a feature's state is
  written when it changes (`@atlas/feature` by hand, S5's fights themselves) and every window
  over it redraws; the applier keeps the live state through a pack update (rule 8).
- **Rasterizer folds**: a door drawn beside its wall (the threshold's far side, or just off a
  floor's tile) cuts the nearest wall cell; a floor is void beyond its building in a composite,
  and the ground is void under one, so a walk changes surface at the door.
- **Not built**: elevation digits on request (the class glyph is the band), the room-scale and
  neighborhood-scale views on the feed (`+atlas` keeps its own text map).

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

**Ledger (landed 2026-09-10).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test` on `mush-sim` (with `testkit`), `mush-atlas`,
`mush-propagation`, `mush-tactics`, `mush-scenes`, `mush-core`, `mush-proto`, `mush-ansi`,
`mush-net-ssh`, `mush-gateway`, the dependents, the contract tests (the panic budget —
`mush-atlas` still at zero sites — the restriction-lint table, the legacy-handler allowlist
with `@atlas` gaining `/los`; no new handler), `mush-server` and `mushd` whole. Twenty new tests:
every row the text names, the slope limits and the climb, the four sight cases, the
direct trace's first blocker and cover, the arc over a wall and under a canopy, the beam's
attenuation, hearing by distance, walls, features and the vacuum, the path round a wall and its
budget, the same trace twice (D), the same cells read for a hit and a miss (O), the core row
refused (X), propagation's range and occlusion over the lot, `+move`'s price, the slot filled
once. Deviations from the text above, each with its reason:

- **The classes stay the S1 enum, extended** (`woods_light`, `woods_heavy`, `water_deep`,
  `ice`, `regolith` join it; `water` is the shallow kind), with `props` giving each its cover,
  opacity and medium. A cell is one byte of class, so a ruleset's own class cannot yet be a
  cell; the **table** accepts a ruleset's class and locomotion rows by name
  (`LocomotionTable::extend`) and refuses a core pair, which is the X lens's actual test. Cells
  of ruleset classes are S7's, with the TacSim driver that needs them.
- **The vocabulary is one list**: `mush_atlas::physics::{BASE_MEDIA, BASE_LOCOMOTIONS,
  BASE_TAGS}`; `MobilityVocabulary::base` in `mush-tactics` is built from them (the crate
  already depended on the atlas), so `biped` joins the tactics base too. The registry type
  itself did not move — nothing in the atlas needs it.
- **Costs are hundredths, integers**; the table's rows are per-locomotion defaults with named
  classes rather than a full cross product, and each locomotion has a **climb** (`climb_cm`: a
  kerb, a doorstep, a dock — built ground a step up to that height climbs at half again the
  cost, whatever the slope) because the lot's loading dock stopped the first walk as a
  120 % slope. Tags amend rows (`amphibious`, `submersible` open water; `jump` doubles the slope
  limit).
- **The geometry reaches propagation through `mush-sim`**, not an atlas edge:
  `WindowGeometry` is a slot beside `RoomResolver`, filled by the atlas at boot;
  `PropagationContext::build` consults it before the graph's adjacency refusal (two ends the
  room graph calls separate places may still share a window) and takes range and occlusion from
  it. `mush-propagation`'s crate edges are unchanged. `Medium::from_medium_id` and
  `medium_ids` map onto the base media.
- **Traces are single-surface**: two ends on different surfaces of one window (a floor and the
  ground) are occluded with the grid's range, and the sight, fire and hearing helpers answer
  `None` for them; a line through a doorway between surfaces is S7's with the multi-storey
  fights.
- **`@atlas/los <a>=<b>`** is one read switch (staff), so the traces are visible by hand before a
  ruleset reads them; `direct_fire_between`, `sight_between` and `hearing_between` are the
  ruleset-facing reads over two objects' poses.
- **Not built**: `[atlas].path_max_cells` is threaded through config and read by no command
  yet (`path` is a library query for S6's NPC walks); elevation digits on the map.

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
Heron, Janis Ian, Phil, Duvall and Norbury front the abilities of doc 14 §2.6; Aaron drives and
fronts nothing.

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

**Ledger (landed 2026-09-10).** Gates: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test` on `mush-core`, `mush-npc`, `mush-atlas`,
`mush-worldgen`, `mush-jobs`, `mush-market`, `mush-weather`, `mush-ruleset`, `mush-server` and
the dependents, the contract tests (the panic budget, the restriction-lint table, the allowlist
with `@agent`'s note carrying `/audit` and the `interfaces` field; no new handler), `mush-server`
and `mushd` whole. Eighteen new tests, across the seam (the manifest offered and the answer
spoken as the asker, a refusal in the asker's words, an ambient turn holding no one's
authority, a gone asker refused, an uncomposed provider skipped, the fallback naming the
command, the field set and the audit read, the help topic), the lot (the Production Office
absent for a player and present for staff from the same NPC, a DARK player absent from
`who.nearby`, the bus on the road, four reads persisting nothing), the desk (a proposal owned
by the asker and invisible to another, a ticket filed and shown to its opener alone, a status
persisting nothing), the weather (the cell read without a roll, indoors refused), the market
(the projection read, a bad listing refused) and the composition (a ruleset brings its rules
interface). Deviations from the text above, each with its reason:

- **The provider trait lives in `mush-core`** (`InterfaceAbilityProvider`, registered by id
  through `Registrar::interface_provider`, read through `Registry::interface_provider`), not
  beside the ruleset grounding in `mush-npc`: the crates that front abilities — `mush-weather`,
  `mush-market`, `mush-jobs`, `mush-ruleset` — cannot depend on `mush-npc` (the ruleset is
  its dependency). The NPC's `AgentProfile.interfaces` names provider ids; the turn merges
  their manifests into its tool set (`TurnSnapshot.interfaces`) beside the ruleset's.
- **`AbilityContext { npc, asker, principal, powers }` is `AbilityCall { npc, asker }`** and
  the command context itself is re-actored to the asker for the call: every provider reads
  through the projection its own command uses with the asker as the viewer, so principal and
  powers are whatever that projection already derives, never a second copy.
- **The answer is spoken.** An interface ability executes on the world loop at re-entry, like
  `wod_roll`, after the model's turn; its reply (or its refusal, which is the asker's own
  command's word) is what the NPC says. The model requests, the world answers; nothing is fed
  back into the same turn's tool loop.
- **The audit ring is the NPC module's** (`NpcHostState.interface_audit`, `@agent/audit`),
  not the LLM audit sink, which lives off-loop; each row is `(npc, asker, ability, input,
  answered|refused)`.
- **The rules interface is a small module** (`mush_ruleset::interface::RulesInterfaceModule`)
  the server composes beside the ruleset, snapshotting the chargen stages at composition: a
  ruleset registers its own commands in `register` and cannot hand itself out as a provider.
- **`who.nearby` is `look`'s answer**: the asker's room's occupants as they would see them
  (DARK and disconnected absent). The "advertised" presence projection is D-14-1's excluded
  scope; when it lands, this ability reads it.
- **`weather.forecast` reads the transition table** (every season's row for the current state,
  summed; the likeliest other state) and rolls nothing: there was no forecast API to front.
- **`market.quote` writes the asker's own quote record**, reserving nothing — the one write the
  economist makes, a proposal-shaped record under rule 2; `market.list` walks `api::visible`
  as the asker, whose hidden-seller property is that API's own test.
- **The authority test is the Production Office through Susie**, not a hidden control region
  through Gretchen: a control region's hiding is not an input to `atlas.describe`, and Gretchen
  fronts nothing (the packs name Susie, Karen Smith, Regina, Cady Heron, Janis Ian, Phil, Ron
  Duvall and Sharon Norbury). The lens is the same: one lore, two askers, two answers.
- **The packs are at version 3** with `interfaces` on eight profiles; the lobby's requirement
  on the OOC Area moves to 3 with it; the digests are re-pinned.

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

## 6. Decisions taken by this plan (acknowledged by the owner 2026-09-09; ★ marked the owner calls, now decided)

| # | Decision |
|---|---|
| D-P-1 ★ | `ruleset-tacsim` keeps its own seed latch through S1–S6 and folds onto `mush-content`'s in S7 — **decided** |
| D-P-2 | `mush-content` before `mush-sim`, `mush-atlas` after it, in `server_modules_with` |
| D-P-3 | Starter packs are `include_str!` fixtures in `mush-atlas` with digest-pinning tests |
| D-P-4 | The journey document moves under the `atlas` family; `route.rs`'s runtime (`Journey`, `Leg`, the sweep, `@route`, `+intercept`) is deleted in S2, not deprecated. **Amended at S2 (owner, 2026-09-09):** the *carrier* journey (`journey.rs`, `@journey`) is **layered, not replaced** — it stays for placement-graph carriers until S7's TacSim fold, where one record absorbs both; S2's `JourneyRec` carries characters, objects and room-model vehicles. Reason: `CarrierJourney` is wired into `mush-sim`'s placement gate, quarantine, the pulse authority's `Promoted` state, the interception raise, boot hydration and about a hundred test references, and moving it under the atlas needs a core-level seam `mush-sim` would read the atlas's slot through; the plan's S7 fold is where that seam belongs |
| D-P-5 | Theme and weather move in S2b, not S1, so S1 has no behavioural change outside new verbs |
| D-P-6 | The tool loop lives in `mush-llm` and `mush-npc` becomes its first caller (one loop, two users) |
| D-P-7 | `+move` is a generic atlas verb gated by the scene turn, not a ruleset verb |
| D-P-8 | The composite stays at `mod:tactics:map:<domain-id>` (doc 06 §9.1's key) with owner `atlas` |
| D-P-9 | Feature-state write-back rides the domain tick's commit, never a second timer |
| D-P-10 ★ | Aaron Samuels drives the bus from S2; Norbury (docent) and Duvall (front office) ship in S6 — **decided**: the packs carry ten cast members |

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
- D-P-1 and D-P-10 were the owner's calls; both are decided (§6).

## 9. What this plan hands back

- **Doc 14 §8's supersessions become deletions** as each cut lands: `route.rs` (S2), the ZMO
  theme read (S2b), `set_map_blob` and `toy_map` (S4), the tactics stub's hex vocabulary (S7).
- **14c (presence) and 14d (faction places)** start after S6, on `who.nearby`'s eligibility set
  and on `SiteRec.property` respectively.
- **The hired car** (doc 07 §9 `MobilityJob`) layers on S2's journeys and S6's interface
  abilities: a dispatcher NPC fronting `travel.request`.

## 10. Changelog

- 0.1 (2026-09-09): first draft, from doc 14 Draft 0.1; S7 gains the campaign ephemeris (doc 10 T17) the same day; §6's decisions acknowledged by the owner the same day.
- 0.2 (2026-09-09): S1 landed — `mush-content`, `mush-atlas`, the starter packs, `[game].starter_area`, `[atlas]`; its ledger and verify fold recorded under §3.1; §8's tile-size question closed by measurement.
- 0.3 (2026-09-09): S2 landed — one travel system in `mush-atlas` (journeys for characters, objects and the room-model bus; `+travel`, `+stakeout`, `+intercept`, `@atlas/journey`; timed portals; the clock floor and cap), `route.rs`'s runtime retired; D-P-4 amended (the carrier journey is layered until S7); its ledger and verify fold under §3.2; the starter packs at version 2.
- 0.4 (2026-09-09): S2b landed — theme, weather, control regions and property key on the atlas (D-14-28): `Theme` in `mush-atlas` with a world theme and `@atlas/theme`; weather cells on neighborhoods and regions; `@region/atlas`, `@property/site`; `@theme` retired; its ledger under §3.3.
- 0.5 (2026-09-10): S3 landed — the building agent and hydration: `mush_llm::agent` (the one tool loop, the NPC turn on it), `mush_atlas::validate` (doc 14 §4.3 as typed findings, shared by `@atlas/pack plan` and the agent), `mush_atlas::hydrate` (standing, requests, activation, the BUILD queue, canon events), `mush_worldgen::{proposal, plan}` (proposals, the planning run, `+build`, `@atlas/propose`); `[worldgen]`'s five knobs and the `worldgen_plan` route; its ledger under §3.4.
- 0.6 (2026-09-10): S4 landed — the ledger under §3.5 (the composite through the atlas's own async completion, the window document in the atlas's namespace, `WindowRec` and `Pose` in `mush-sim` with the resolver slot, the shape-driven text fallback, the map pane, `+map` as one command); the rasterizer's door snap; `[atlas].window_margin_m` and `window_max_cells`.
- 0.7 (2026-09-10): S5 landed — `mush_atlas::physics` (the vocabularies, the locomotion table as digest-pinned data with climbs, `move_cost`, the bounded path) and `mush_atlas::trace` (sight, the direct, indirect and beam traces, hearing), `mush-sim`'s `WindowGeometry` slot and propagation on the metric, `+move` priced, `@atlas/los`; its ledger under §3.6; `[atlas].path_max_cells`.
- 0.8 (2026-09-10): S6 landed — `mush_core::InterfaceAbilityProvider` and its registry, `AgentProfile.interfaces`, the turn's merged manifest, the asker-threaded invoke with the answer spoken, the audit ring and `@agent/audit`, the fallback naming the command; the providers `atlas`, `travel`, `weather`, `market`, `build`, `request`, `rules`, `help`; the packs at version 3; its ledger under §3.7.