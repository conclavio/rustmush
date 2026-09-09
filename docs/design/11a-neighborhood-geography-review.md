# 11a — Neighborhood geography and authoring: design review

**Status:** review, 2026-09-09, on main `0bc5e92`. Written for the owner. Nothing here is built;
§5 is a proposal, §8 the decisions it needs, §9 the edits owed to doc 11 and its fixtures.

**Subject.** `docs/design/11-neighborhood-geography-and-authoring.md` (doc 11), its
[brief template](neighborhoods/TEMPLATE.md), and the [urban](neighborhoods/examples/lantern-ward.md)
/ [rural](neighborhoods/examples/mill-creek.md) examples, as committed at `0bc5e92`.

**Owner premises for this review** (stated for this pass, in the owner's words paraphrased):

1. Small, disparate, fully built neighborhoods reached by a travel verb (`+uber`, `+car`, whatever),
   not a huge grid of rooms nobody uses.
2. Because neighborhoods are focused, bar-level and street-level tactical maps can be part of the
   neighborhood's design; neighborhoods sit on a real grid relative to one another with the space
   between existing "off-screen".
3. Hydrate-on-demand: players need a bookstore the neighborhood's vibe implies but nobody built;
   they describe it, the building agent stands it up, they play there, the room is marked pending
   review for a human builder who knows the neighborhood, then it is persisted permanently.
4. Elevation and terrain should be integrated so the map and the room system are one coherent thing.
5. **No migration burden.** The game is still being built; concepts may be replaced outright.
6. Starfield is the reference for movement between scales of zones — discrete, bounded places at
   surface, orbital, system and interstellar scale, with ships carrying players between disparate
   locations and the space between abstracted — for the Expanse-scale corridor and for the
   land-based game alike (a car or a train is the same pattern at city scale).

Premise 5 changes the review's stance: the tree is cited below as *evidence of what exists*, never
as a compatibility constraint. Where a landed record is the wrong shape for premises 1–4, the
recommendation is to replace it, keeping only the invariants that were expensive to get right.

**Sources, by symbol.** `crates/mush-core/src/mechanics.rs` `CanonicalId`,
`validate_canonical_identifier`, `DefinitionDigest`; `resource.rs` `ReservationRecord`,
`ReservationBook`; `clock.rs` `ClockRef`, `ClockStamp`, `World::world_ic_now`; `module.rs`
`Ctx::set_map_blob`, `HookPoint::MoveAttempt`, `JobQueueSpec`; `obj.rs` `ObjKind`, `Obj`;
`commands/movement.rs` `traverse_exit`. `crates/mush-sim/src/data.rs` `SpatialParent`,
`PlacementRecord`, `DomainRecord`, `DomainClock`, `SpaceAnchor`, `InteriorSpaceRecord`; `graph.rs`
`project`; `domain.rs` `MAX_ACTIVE_DOMAINS`, `PromotionTrigger`; `route.rs` `Journey`, `Leg`,
`RouteDef`, `LegDef`, `MAX_LEGS`, `RouteCatalog`; `journey.rs` `CarrierJourney`, `advance_due`,
`reanchor`; `commands.rs` `route_create`, `arrival_parent`, `build_route`, `pose_from`,
`PASSIVE_DRIVER`; `transition.rs` `Transition`, `Plan`. `crates/mush-tactics/src/lib.rs`
`TacticsStub`, `toy_map`; `mobility.rs` `MobilityMode`, `ModeRules`.
`crates/mush-propagation/src/context.rs` `Geometry`, `ADJACENT_RANGE`.
`crates/ruleset-tacsim/src/pack.rs` `PackManifest`, `PackKind`, `PackRequirement`, `LocationDef`,
`RouteEntry`, `PackLeg`, `pack_digest`; `seed.rs` `SeedPlan`, `PlannedObject`, `apply`;
`record.rs` `SeedApplication`; `fixture/tacsim_corridor.json`. `crates/mush-worldgen/src/theme.rs`
`THEME_ATTR`, `theme_chain`, `resolve_theme`; `commands.rs` `walk_building`, `MAX_UNZONED_HOPS`,
`apply_exit`, `write_accepted_fields`; `data.rs` `BuildingBrief`, `StoredDraft`.
`crates/mush-jobs/src/data.rs` `Job`, `JobPayload`, `JobStatus`; `api.rs` `PayloadHandlerFn`,
`open_job`; `facts.rs` `JobsFactProvider`. `crates/mush-llm/src/service.rs` `LlmFeature`,
`CallCtx`, `KillSwitch`; `provider.rs` `BudgetScope`; `budget.rs` `ScopeLimits`; `audit.rs`
`AuditSink`. `crates/mush-npc/src/knowledge.rs` `Secrecy`, `LorePool`, `familiarity_of`;
`prompt.rs` `render_world_knowledge`; `lib.rs` `Stimulus::AmbientTimer`.
`crates/mush-faction/src/data.rs` `Region`, `RegionId`, `ControlRecord`.
`crates/mush-housing/src/record.rs` `Property`. `crates/mush-org/src/record.rs` `OrgScope`,
`Delegation`; `plan.rs` `ExpectedRevision`. `crates/mush-economy/src/plan.rs` `EconomyPlan`;
`record.rs` `ApprovalRequest`, `IdempotencyKey`. `crates/mush-scenes/src/data.rs` `SceneRecord`,
`ConsentVote`, `scene_ic_time`. `crates/mush-session/src/manager.rs` `SessionManager::who`.
`crates/mush-canon/src/data.rs` `CanonDraft`, `CanonEvent`. Docs 03 §5, 06 §§5–7, §9.1, §10, D1;
07 §9; 09 C8; 09b; 10 §2, 1A, 6A, T1, §16 item 9, §20, C16A/C19A/C20A; 10b §3 cut table, cuts 1
and 6; 12/13; `PLAN.md` treaty rulings 3, 4, 8; `docs/help/{worldgen,spatial,route,jobs,faction,
npc,llm}.md`; `docs/scenes_factions_knowledge.md` §3.5. Cites are **path + symbol** and **doc §**;
line numbers are deliberately absent (08i §8's rule).

---

## 1. Verdict

The direction is right and it composes with what is built. *Continuous geography, selectively
authored room maps* is doc 10 §1's "never simulate the entire solar system at tactical resolution"
applied to a city, and it sits naturally on doc 06's single authoritative placement graph. The
constraint discipline is the document's best property: secrets never reach prompts, travel never
teleports, live state is never rewritten from source, review is independent of existence, no
automatic teardown. Those should all survive.

Five things stop the document from being buildable as written.

1. **It specifies almost no data.** Every sibling doc (06, 08, 09, 10, 13) fixes record shapes,
   identity types, bounds and the storage lane; doc 11 names concepts (region, locality,
   neighborhood, site, structure, room, connection, manifest, hydration request) and gives none of
   them a record, an id type, a namespace or a cap. §5 below proposes the records.
2. **The tactical tie-in stands on a terrain layer that does not exist in any form.** The tree has
   no coordinate, frame, transform, footprint, resolution, elevation, cover, line of sight or
   terrain anywhere: `DomainRecord` is `{id, host, driver, status, clock, presentation_location,
   revision}`; the only driver `@domain/create` accepts is `sim.passive` (`PASSIVE_DRIVER`); a pose
   is opaque JSON (`SpatialParent::Direct { pose: RulesValue }`, built by `pose_from` from raw
   text); the only map is a hard-coded 2×2 toy hex blob (`toy_map`) stored through
   `Ctx::set_map_blob`, which core "stores and returns, never reads"; `mush-propagation`'s
   `Geometry` is explicitly non-metric (range 0 in the same place, `ADJACENT_RANGE` = 2 elsewhere).
   No E7 cut (10b §3, cuts 1–11) builds terrain. Doc 11 §5 and §8 speak of "the selected driver",
   "declared resolution" and "a physically valid tactical terrain asset" as if they were seams to
   extend. They are a workstream to start — and, per premise 4, the one that decides whether rooms
   and maps become one system or two. §5 is that proposal.
3. **It under-cites the tree where the tree already solved its problem.** Four landed mechanisms
   answer needs doc 11 declares net-new: `CanonicalId` + `DefinitionDigest` (stable content ids and
   content hashes); the E7 rules pack + seed latch (`PackManifest` with digest-pinned `requires`,
   `SeedPlan` → `SeedApplication.placed: BTreeMap<CanonicalId, ObjId>`, crash-safe, never re-run —
   exactly §7's "content-ID → runtime-object mapping" and "unchanged manifest applied twice is a
   no-op"); `RouteDef`/`LegDef` + `CarrierJourney` (timed, roomless, IC-clock legs with an
   idempotent cursor, hazard/interdiction bands per leg, promotion into a live domain, re-anchoring
   across rate changes — §4's journey contract items 2–5 almost verbatim); and C8
   `ReservationRecord`/`ReservationBook` (§8's "atomically reserve … capacity … and a footprint").
   One §10 claim is misleading: "current `@route` cannot traverse roomless geography" is true of
   `route.rs`'s room-list `Journey`, but `journey.rs` already flies legs whose waypoints are any
   live object and whose arrival is a domain host or interior (`arrival_parent`, `build_route`).
4. **Identity and dependency conventions in the fixtures cannot be represented or bootstrapped.**
   `CanonicalId` admits only `[a-z0-9_.]` (`validate_canonical_identifier`); every example id uses
   `-` and cross-neighborhood references use `/`. The two examples each list the other's rooms and
   the shared corridor as activation-blocking `external_dependencies`, so neither can activate
   first (§4 H3).
5. **Two policy reversals and one scope conflict are unrecorded.** Stage B lets an LLM propose
   topology, reversing the shipped rule "worldgen describes; it never digs" (doc 03 §5.3, help
   worldgen, `apply_exit` refusing unknown exits) without saying so or bounding it. Hydration
   quotas are account-scoped in a design where the account is deliberately not a principal and may
   appear only as an audit-only oracle (doc 10 C1A, R3, R7), and where LLM budgets are per `ObjId`
   (`BudgetScope::Actor`). And the Markdown-with-YAML brief has no parser and no canonical
   encoding, while every digest in the tree is over canonical JSON (10b cut 1's stated reason for
   choosing JSON packs over TOML).

None of these is an argument against the direction. Under premise 5 they are cheap to fix now and
expensive later: the fixes are the records in §5, the reuse in §3, and the decisions in §8.

## 2. What doc 11 proposes, in one table

| § | Proposal | Status of the underlying mechanism |
|---|---|---|
| 2 | Vocabulary region → locality → neighborhood → site → structure → room; zoom ≠ simulation scale | Nothing exists; zones and faction regions are the only groupings |
| 3 | Metric region frames, neighborhood transforms, footprints, anchors, stable ids, authored/implicit, 8–20 room density | No coordinates anywhere; `CanonicalId` exists with a narrower byte class than the fixtures use |
| 4 | Local exit / boundary connection / journey connection; a five-step journey contract | `route.rs` room journeys (staff) and `journey.rs` carrier journeys over `RouteDef` legs (IC clock) exist; exits carry no distance or time |
| 5 | Tactical domains reference footprint, transform, terrain revision, resolution; many-to-many coverage; D1 resolver; durable damage on stable features | `DomainRecord` has none of those fields; one passive driver; no terrain, cover, elevation or LOS; D1's `PresentationResolver` is unbuilt |
| 6 | Open-to-play presence, coarsened, consent-based | Nothing; `SessionManager::who` has no location; scene consent votes cover end/outcome/rewind/nudge only |
| 7 | Stage A saved proposal + survey rounds + evaluate + approve; Stage B typed build manifest, staging, diff/apply, three-way merge | `@gen`/`@gen/building` draft-review exists (prose only, per-building `BuildingBrief` palette); no manifest, staging or diff; E7 packs + seed latch are the un-cited precedent |
| 8 | Player hydration: find → reserve → generate → validate/activate as `active + pending_review` → persist + queue; quotas; duplicates; Git reconciliation | No registry, no import/export, no YAML; C8 reservations, `IdempotencyKey`, jobs queues, canon ledger exist to build on |
| 9 | Faction places: LLM review then staff decision; capability/resource schedules | `mush-org` scopes/delegations, `EconomyPlan` + hash-bound `ApprovalRequest`, `Property` exist; audience-class visibility (E4b2/T6) is the report-filter precedent |
| 10 | Seam table and a seven-step build order | Partly accurate; see §3 |

## 3. Claims against the tree

Every "existing" or "reuse" claim in doc 11 §10 and the body, read against `0bc5e92`.

| Doc 11 claim | Verdict | Evidence |
|---|---|---|
| `mush-core` ordinary rooms, exits, locks, zones | **implemented** | `ObjKind::{Room, Exit, …}`; `Obj.zone: Option<ObjId>` (a zone is a Zone Master Object, there is no `ZoneId`); `LockType::Zone` delegation; exit lookup room → zone → Limbo (`commands/movement.rs`). **Exits carry no distance or time**; traversal is instantaneous and vetoed only through `HookPoint::MoveAttempt` |
| `mush-worldgen` theme resolution and reviewed descriptions | **implemented** | `THEME_ATTR` = `worldgen.theme` on any object; `theme_chain` room → zone chain (≤ 8 hops) → `#0`; `resolve_theme` nearest-wins scalars, unioned lists. Describes only: `write_accepted_fields` writes `desc`/`short_desc`/`detail:*`/exit `desc`+`osucc`; `apply_exit` refuses an exit it cannot find |
| "brief/manifest planner, identity mapping, topology validation, staging, diff/apply" as extensions of worldgen | **missing** — and mis-sited | The only "brief" is `BuildingBrief` (concept + palette + one line per room); no manifest, staging, diff, content id or registry in the crate or the repo. The precedent is `ruleset-tacsim`: `PackManifest {id, version, kind, digest, requires}`, `PackRequirement.digest`, `pack_digest` over canonical JSON, `SeedPlan`/`PlannedObject`, `seed::apply` with the `SeedApplication.placed` latch |
| `@gen/building` BFS within a zone, bounded hops if unzoned | **implemented exactly** | `walk_building`: zone-membership or `MAX_UNZONED_HOPS` = 6; `MAX_BUILDING_ROOMS` = 40; the command already warns an unzoned walk "may include rooms outside the building" |
| `mush-jobs` durable queues and typed approval payload handlers | **partial** | `Job`, `JobPayload { handler, data: serde_json::Value }`, `PayloadHandlerFn` run inside `+job/approve`'s batch, `payload_applied`. "Typed" is loose: no schema version, no revision, no content hash, no ticket-linking field. Payload jobs are one-shot; **plain jobs can be reopened** `Resolved → Open`. `JobsFactProvider` refuses revisioned queries. Queues are module-registered (`JobQueueSpec { name, title, min_role }`) |
| `mush-sim` placement, interiors, domains | **implemented** | `SpatialParent`, `PlacementRecord`, `graph::project` (`Obj.location` is a projection), `InteriorSpaceRecord`, `SpaceAnchor::{FixedDomain, MovingObject, Unanchored}`, `DomainRecord`, `DomainClock::{Passive, Pulse}`, `@domain/promote` with `PromotionTrigger`, `MAX_ACTIVE_DOMAINS` = 8, `transition::{plan, apply}`. **No geometry of any kind**; no player verb enters a domain from a room (only staff `@spatial/place` and `egress`) |
| `mush-sim::route` room-list `Journey`; "current `@route` cannot traverse roomless geography" | **implemented; the caveat is half true** | `Journey { traveler, legs: Vec<Leg { to: ObjId, travel_s }>, cursor, … }`, `route_create` requires `ObjKind::Room` legs, staff-only, wall-clock sweep. **But** `RouteDef { id: CanonicalId, endpoints: (ObjId, ObjId), legs: Vec<LegDef { to, base_s, hazard, interdiction }>, … }` and `CarrierJourney` (IC clock, idempotent `cursor`, `advance_due`, `reanchor`, promotion into a domain) already traverse timed legs between objects with no room chain — the shape doc 11 §4 describes as the "future journey extension" |
| `mush-npc` profiles and tiered lore | **implemented, with a caveat the template relies on** | `Secrecy::{Public, Guarded, Secret}`, `LorePool` by tag, secret filtered twice (`resolve_shareable`, `render_world_knowledge`). **`Guarded` is prompt framing, not a server gate** (`familiarity_of` annotates the asker; the model is asked to hold back). No routines: `Stimulus::AmbientTimer` is modeled but never scheduled |
| `mush-faction::Region` with `RegionId` and `zones` | **implemented** | `Region { id, name, note, zones: Vec<ObjId> }`, `ControlRecord`, `@region/create|zone|note`, `+territory`. The code's own comment: "deliberately not the room graph … map-blob areas arrive with the tactical subsystem" |
| `mush-org` scoped authority; `mush-economy` plans/approvals | **implemented** | `OrgScope`, `CapabilityGrant`, `Delegation` (one hop, never widens), `AuthRequest`/`Decision`; `EconomyPlan` with `IdempotencyKey` and expected revisions; `ApprovalRequest` bound to a `plan_hash`, four-eyes re-derived live |
| Tactical domain references footprint, transform, terrain revision, resolution (§5) | **missing** | `DomainRecord` has none; no frame anywhere in `mush-sim`, `mush-tactics`, `ruleset-tacsim` |
| "The selected driver" validates terrain (§8) | **missing** | One driver, `sim.passive`; no registry; no terrain driver |
| Terrain, cover, elevation, LOS, entrances, room ↔ terrain mapping | **missing** | `toy_map` (2×2 hexes with `terrain` strings and `elev` integers) behind `set_map_blob`; `mush-propagation` is non-metric by design and expects "a real metric to replace this" |
| Terrain generation of any kind | **missing** | none, including in worldgen |
| Open-to-play presence, consent to advertise (§6) | **missing** | no feed; `who` is connection metadata; `ConsentVote` is scene-safety only |
| Stable namespaced content ids for world objects; content-id → object registry | **missing as a world concept; precedent exists** | dbrefs only for objects; `CanonicalId` names definitions, packs, routes, clocks; the seed latch maps pack entry → `ObjId` |
| Lots/parcels, site/structure/neighborhood/locality/atlas records | **missing** | none of those words occurs in Rust source |
| "the same travel authority" for local timed movement and journeys (§4) | **missing** | three unrelated paths today: exit traversal (instant, hook-vetoed), `@route` (staff, rooms), `@journey` (carriers, IC) |
| Hydration "content registry", "export/reimport" (§8) | **missing** | no import/export of world content; no Git integration; no YAML/Markdown parser (`toml` is the only config format dependency) |
| `mush-housing` property (implied by "consume someone else's property") | **implemented, room-set shaped** | `Property { anchor, rooms: BTreeSet<ObjId>, tier, landlord: PrincipalId, … }`; title on the anchor, not on land; no parcels |
| Clock contracts for journey/narrative/tactical time (§4) | **implemented** | `ClockRef::{Utc, WorldIc, SceneTurn, DomainTick, Registered}`, `ClockStamp` with generation-checked arithmetic; carrier journeys and the domain pulse run on `WorldIc`; the 6A scene time floor (`ic_floor`) exists |

**Un-cited seams doc 11 should name** (all landed): `CanonicalId`/`DefinitionDigest`; the
rules-pack loader and seed latch; `RouteDef`/`LegDef`/`CarrierJourney`; C8 reservations;
`mush-canon` (activation, review decisions and retirements are canon events); typed state families
and the 09b lane rules (where atlas records live); `SceneRecord.anchor`/`exclusive`/`observers` as
the source of "joinable scene" advertisements (an admission policy does not exist yet); `Property` for the ownership check.

## 4. Findings, ranked

Severity is about cost-if-unfixed at build time, not about the prose.

### HIGH

**H1 — No terrain substrate; the design assumes one.** Doc 11 §5 ("a tactical domain references a
geographic footprint, a transform … terrain revision, and a declared resolution") and §8 step 3 ("a
physically valid tactical terrain asset … validate against the selected driver") describe extension
points that do not exist (§3). E7 cuts 1–11 contain no terrain cut; cut 8's pulse driver owns
`TacticalUnit` only. As specified, a "fully hydrated" site (template §"Player-requested site
hydration": "a site advertised as fully hydrated requires actual validated tactical terrain, not a
placeholder") has no path to ship, and the owner's premise 2 (bar-level maps as part of
neighborhood design) has no data model. *Fix:* adopt one substrate for rooms, terrain and elevation
(§5), and decouple the first hydration slice from terrain (D-11a-6).

**H2 — Identity convention is unrepresentable.** `CanonicalId` (the only stable id type, used by
packs, routes, clocks and definitions) admits `[a-z0-9_.]`, refuses leading/trailing/double dots,
and caps length; the template mandates `neighborhood.replace-me`, `region.replace-me.surface_m`,
and cross-references of the form `neighborhood.lantern-ward/room.east-arrival`. Doc 11 §10 says the
atlas "needs its own geographic identity type/namespace" and then adopts a syntax that would force
a second id type with a second byte class, a second parser and a second digest discipline. *Fix:*
pack-scoped `CanonicalId`s — a neighborhood is a pack (`lantern_ward`), its rooms are entries
(`room.bar`), and a cross-pack reference is the pair `(pack, entry)`, exactly as `tacsim_corridor`
references `tacsim.core` and a facility's `at` names a location in the same pack (D-11a-1).

**H3 — The two example briefs cannot bootstrap each other.** Lantern Ward lists
`neighborhood.mill-creek/room.west-arrival` in `external_dependencies`; Mill Creek lists
`neighborhood.lantern-ward/connection.east-road` and `…/room.east-arrival`. The template says
"missing external references block activation; the builder must not invent a destination."
Neither neighborhood can be activated first. The root cause is ownership: the shared corridor is
authored inside Lantern Ward's brief even though both briefs also depend on a region-level
`corridor.east-road`. *Fix:* connections are region-owned records in a region/corridor pack (the
tacsim corridor's `route` entries already are); a neighborhood brief declares only its access points
(arrival rooms) and which region connections terminate at them; a connection whose far endpoint is
not yet activated is **inactive**, not invalid, and activates when both ends exist (§5.6).

**H4 — Travel is planned as a fourth movement system.** The tree already has three unrelated
movement paths (exit traversal, `@route`, `@journey`). Doc 11 §4 designs a "future journey
extension" on `route.rs`'s room-list `Journey` and states journeys through roomless geography are
unsupported; `journey.rs` supports them today, on the IC clock, with the never-skip invariant, an
idempotent cursor, hazard/interdiction bands per leg (`LegDef`), promotion into a live domain
(`PromotionTrigger::Interception`), and `MAX_LEGS` = 32. Building a parallel mechanism would leave
four. *Fix (premise 5 applies):* one travel system, built from the carrier journey's core and
replacing `route.rs`'s room-list journey outright: a **traveler** is a plain object, a character, or
a carrier root (the current `CarrierJourney` requirement that the root be a placement-graph citizen
becomes one traveler kind, not the rule); a route is an atlas **connection** (§5.6) whose legs are
its checkpoints; local timed exits (the rural farm road) are one-leg journeys under the same
authority; the TacSim-only fields (`drive_class`, `propellant`, `alignment_band`) move into a
ruleset-owned cost policy on the connection. Keep `Journey`'s live-narration, catch-up-in-one-move,
grace-after-resume and `+intercept` behaviours as properties of the one system (D-11a-4).

**H5 — Stage B reverses "worldgen describes; it never digs" without recording it.** Doc 03 §5.3
rejected topology generation; help worldgen and `apply_exit` enforce it; the treaty rulings do not
touch it. Doc 11 §7 Stage B says "an LLM may propose topology in this new planning stage" and lists
validation only as "bounds, budgets, physical connections, reachability, access, and direction
consistency". *Fix:* record the reversal as a decision; make the LLM's output a **typed site plan**
(rooms with footprints, floors, portals, features, kinds — never cells, never exits by name) that a
deterministic validator checks for footprint containment, non-overlap with reserved/private
footprints, portal adjacency, reachability from a public access point, budget, and floor
consistency; and for hydration prefer **template-constrained topology** — a small library of site
kinds (bar, shop, park, flat, workshop) each with an allowed room set — so the validator's surface
and the duplicate check are both bounded (D-11a-8).

**H6 — Quota scope contradicts the account rule and the budget model.** §8: "account-level limits
prevent alts or tabs multiplying the quota." Doc 10 C1A/R3/R7: the account is never a principal;
its id is an opaque, audit-only distinctness oracle, never in a journaled or canon record. LLM
budgets are `BudgetScope::{Global, Feature, Actor(ObjId), Npc(ObjId)}` — there is no account
scope. *Fix:* separate the two limits doc 11 already distinguishes. The **creation allowance** is
a per-account counter kept in the account-integrity lane beside the `AccountOracle` (opaque,
audit-only, never journaled), consulted through a `same_account`-style provider that fails closed;
the **compute** limit is `BudgetScope::Actor` on the requesting character under a new
`LlmFeature::WorldgenSite` route with its own per-feature cap (D-11a-7).

### MEDIUM

**M1 — The manifest and apply pipeline is already half built, in the wrong crate to ignore.**
`PackManifest` (id, version, kind, digest, digest-pinned `requires`), `pack_digest` over canonical
JSON (a reordered table does not change the digest; the declared digest is compared against the
content's, never self-referential), `SeedPlan` → `apply` with the `SeedApplication.placed` latch
(objects and latch in one `WriteBatch`; a partial seed is latched honestly and never re-run;
`@tacsim/seed … dry|commit`). Doc 11 §7's manifest wants exactly this plus **update** and **retire**
diffs and a three-way merge against live edits. *Fix:* lift the pack loader and latch out of
`ruleset-tacsim` into a neutral crate (`mush-content`), add `PackKind::Neighborhood`, and extend the
latch from create-only to create/update/retire with per-entry revisions and reference checks.
The digest gives "source content hash" for free; `Archive accepted output` is "store the pack".

**M2 — No storage lane is named.** Doc 09b: every new mutating mechanic starts as a provider plus
a typed family; legacy attribute lanes may not mutate K-owned families. Doc 11 never says where a
neighborhood, site, connection, tile or room-membership record lives. *Fix:* an `atlas` typed
family (`mod:atlas:*`), room membership as a record keyed by `ObjId` (never an attribute on the
room), tiles as bounded immutable-per-revision documents, and the K3 bundle limits respected by
the compositing step (§5.8).

**M3 — The brief format has no canonical encoding.** "Markdown with small YAML front matter" has
no parser in the tree; 10b cut 1 chose JSON packs over TOML because the digest's canonical encoder
*is* JSON and a second front end would add a second canonicalization. *Fix:* the durable, digested
artifact is canonical JSON (the pack); Markdown+YAML is an **export rendering and an import
front end** whose parse must be byte-stable into that JSON before digesting (D-11a-3). Prose
sections become string fields; tables become entries.

**M4 — Staff submissions need a typed, revision-bound, linkable ticket.** `JobPayload.data` is an
untyped `Value`; there is no schema version, revision, digest or `supersedes` field on `Job`; plain
jobs are re-openable. Doc 11 §7 requires "retries must create one submission", "a pending ticket
must never silently switch its decision target", and "terminal jobs are linked to new revision
tickets". *Fix:* a `build.proposal` payload carrying `{proposal, revision, brief_digest,
schema_version}` (payload jobs are one-shot, which is the semantics wanted), a `BUILD` queue
registered by the authoring module with `min_role` and a `neighborhood` tag for routing to the
neighborhood's reviewers, and the `supersedes` link kept on the proposal record, not the ticket.

**M5 — NPC knowledge assumptions.** (a) Neighborhood-wide lore works today through `LorePool`
tags, but the template's "guarded facts and intended sharing conditions" rest on prompt framing
(`render_world_knowledge` asks the model to hold back; nothing filters), so a guarded fact is a
tone instruction, not a boundary — only `Secret` is enforced. Say so in the template; anything
that must not leak is `Secret` plus a staff/scripted revelation. (b) A hydrated NPC is a standing
LLM cost with its own daily ceiling; default hydrated NPCs to tier `Extra` and `enabled = false`
until review, or count their spend against the requester's actor budget. (c) No routines exist;
the template's "routine/allowed locations" is intent only, as it says — keep it a typed field so a
scheduler can consume it later.

**M6 — Presence has no sources yet and one privacy trap the design half-sees.** No consent flag,
no location in `who`, no feed. The design's "cached, coarsened signal" needs: a per-character
`advertise_open_play` setting; an eligibility set computed on a `Coarse` tick, not on arrival
events; **hysteresis** on bucket transitions so a single arrival/departure cannot flip "quiet" →
"some open play" and back; suppression below a floor; and a `Panel` feed with an SSH text fallback
(doc 06 §10 records that generic panels render on web only). Joinable scenes are a property of
`SceneRecord` (non-exclusive, plus an admission flag it does not have yet), not a second registry.

**M7 — Three grouping mechanisms with no stated derivation.** Zones (theme, lock delegation, zone
exits), faction `Region.zones` (control), and atlas neighborhoods (geography) will coexist. Doc 11
says they are not synonyms and stops. Under premise 5 the cleaner cut is: **theme resolution moves
to the atlas hierarchy** (room → site → neighborhood → locality → region → `#0`), zones keep only
lock delegation and zone exits, and `mush-faction::Region` gains `RegionRef::Atlas(…)` so control
records can name a neighborhood or locality directly (D-11a-9). Whichever way: a neighborhood may
own the zone it maintains, and the direction of derivation must be written down.

**M8 — Room anchors are not enough to place a combatant.** §5: "entering combat chooses a legal
precise pose from the actor's existing location and access point; a room centroid must not place
everyone together." The template gives each room one anchor and each site a rectangle; the
examples then say "do not treat room anchors as combat spawn points" without a field that carries
the alternative. *Fix:* per-room footprints and per-portal geometry (§5.4); entry pose = a free
cell in the room's footprint adjacent to the portal used; exit room = the room whose footprint
contains the pose on that floor, else the domain presentation room — which is D1's promised
`PresentationResolver`, made concrete.

**M9 — Fixture geometry does not pass its own rules.** Lantern Ward: `site.last-tram` (x 240–300,
y 380–430) and `site.courtyard-flats` (x 280–340, y 420–480) overlap in x 280–300 / y 420–430;
`room.courtyard` [300,400] lies on the bar footprint's boundary and `room.stairs` [300,420] on the
corner both sites share. Every exit distance in both fixtures checks out against the anchors, the
corridor arithmetic is consistent ([600,200] → [6200,200] = 5 600 m; Mill Creek's local [0,200] is
regional [6200,200]), and room/exit counts match the budgets — so the fixtures are close. *Fix:*
move the flats to x 300–360, restate the bar as x 240–300 / y 380–420, and add "site footprints do
not overlap unless a shared boundary is declared" to the validator and the acceptance list.

**M10 — Faction review lacks a benchmark, which the design admits; the report filter has a
precedent it does not cite.** The advisory LLM review of "combined power" has nothing to compare
against. *Fix:* the capability schedule is expressed in supported definition ids with numeric
limits, and a deterministic comparator flags rows above the approved exemplar band; the model
comments on the diff and never on the whole. For "the reviewer receives only authorized context",
cite and reuse the E4b2/T6 audience-class visibility (`EconomicVisibility`) rather than a bespoke
redaction.

**M11 — Doc hygiene.** Three documents carry the number 11 (`11-neighborhood…`, `11-h-…`,
`11-tacsim-setting`). The README row has no status or date. The doc bundles seven workstreams. *Fix:*
renumber to 14, split into 14 (world model: geography, travel, terrain), 14a (authoring pipeline
and hydration) and 14b (faction places), and add a records section to 14 (§5 of this review is a
draft of it).

### LOW

**L1 — "Region" collides with `mush-faction`'s `Region`** (`@region/create`, `+territory`,
`docs/help/faction.md`). Keep the geographic word if the atlas commands live under `@atlas/…` and
prose says "control region" for the faction record; or rename the geographic tier. Owner's call
(D-11a-10).

**L2 — Journey clocks.** §4 defers "clock and compression policy" and "exact WoD pacing". Pin the
mechanism now: journeys and timed exits run on `WorldIc` (as carrier journeys and the domain pulse
do), durations are IC seconds from the connection's cost policy, the campaign rate policy (10b
cut 11) sets OOC↔IC, and `journey::reanchor` covers rate-generation changes. Only the WoD numbers
stay open. The 6A scene time floor already exists (`SceneRecord.ic_floor`), so arrivals into a
paused scene have their rule.

**L3 — `@gen/building` site boundaries.** The gap is real and the code already warns about it;
with §5's `RoomGeo.site`, the batch walk becomes "rooms of this site" and the hop heuristic goes.

**L4 — Property is rooms, not land.** `Property.rooms` answers "does this room belong to someone";
"does this footprint consume someone's land" needs the site's `property` link (§5.3). The template's
"do not invent ownership grants" is consistent with `@property/create` minting title only on the
anchor.

**L5 — Template front matter.** `rotation_degrees` with an axis-aligned polygon is fine; add the
elevation datum (`z` of the local origin relative to the region DEM) and a `floors` convention so
premise 4 has a home in the brief; replace `external_dependencies` strings with `(pack, entry,
digest?)` requirements.

**L6 — Template site kinds are free text** ("building/park/farm/compound"). The fit check, the
duplicate check and the hydration policy's "eligible site kinds" all need a controlled `site.kind`
taxonomy (`CanonicalId`s registered like other definitions) plus free-form function tags.

**L7 — The command sketch.** `+build/*` is fine as a proposal surface. Premise 1 also needs the
player travel verb the design describes but never names; one verb, `+travel <destination>[/mode]`
(with `+uber`-style aliases registered by the ruleset), listing only known destinations and modes,
routed through the one travel authority of H4. Hired transport is doc 07 §9's `MobilityJob` on top.

## 5. One substrate: rooms, terrain and elevation as one system (proposal)

Premises 2 and 4 ask for a map and a room system that are the same thing seen at different zoom.
The tree offers a clean slate (§3: no geometry anywhere) and premise 5 removes the reason to stack
a metric layer on top of opaque poses. The proposal: **one metric substrate in region coordinates;
rooms, exits, structures, terrain, elevation, connections and tactical domains are all records on
it; drivers sample it, never own it.**

### 5.1 Frames and elevation

- A **region** declares its frame: metres, `+x` east, `+y` north, `+z` up, an origin, and an
  optional geographic reference (never inferred from an image). Every other coordinate in the
  region is in this frame; neighborhoods carry a rigid transform (origin, rotation) only as an
  authoring convenience and are stored resolved.
- **Elevation is a field, not a per-room number.** A region carries a coarse heightfield (tiles of
  `elev_m: f32` at, say, 10–25 m); an authored neighborhood carries fine tiles (1–2 m) over its
  footprint; a structure carries `floors: [{ index, z_offset_m, height_m }]`. `ground(x, y)` samples
  the finest tile covering the point; a room's `z` is `ground(anchor) + floor.z_offset_m` indoors
  and `ground(x, y)` outdoors; an exit's `dz_m` is the difference of its endpoints and feeds the
  local time policy (a stair is a portal with a `Stair` mode and a `dz`, not a room).
- The **background** of a region (everything outside any tile) is a surface class plus the coarse
  heightfield. Off-screen space is therefore never empty: a corridor checkpoint at regional
  [3400,200] has a ground height and a surface class the moment it is needed.

### 5.2 Records (atlas typed family; every id a pack-scoped `CanonicalId`)

```rust
pub struct RegionRec      { id, name, frame: Frame, background: SurfaceClass, dem: TileSet, revision }
pub struct LocalityRec    { id, region, name, kind: LocalityKind, footprint: Polygon2, revision }
pub struct NeighborhoodRec{ id, region, locality, name, setting, representation: Authored|Implicit,
                            footprint: Polygon2, room_budget, hydration: HydrationPolicy,
                            zone: Option<ObjId>, reviewers: Vec<PrincipalId>, revision }
pub struct SiteRec        { id, neighborhoods: Vec<NeighborhoodId>, name, kind: SiteKind,
                            footprint: Polygon2, entrances: Vec<PortalId>, property: Option<PropertyId>,
                            origin: Authored | Hydrated { request }, review: ReviewState, revision }
pub struct StructureRec   { id, site, footprint: Polygon2, floors: Vec<Floor>, revision }
pub struct RoomGeo        { room: ObjId, id, neighborhood, site: Option<SiteId>,
                            structure: Option<StructureId>, floor: Option<i16>,
                            footprint: Footprint, kind: Outdoor|Indoor|Circulation, revision }
pub struct Portal         { id, exit: Option<ObjId>, from: Endpoint, to: Endpoint,
                            geometry: Segment3, mode: Door|Gate|Stair|Ramp|Ladder|Hatch|Open,
                            distance_m: u32, dz_m: i32, time: TimePolicy, visibility, revision }
pub struct Connection     { id, region, kind: Boundary|Journey, endpoints: (AccessPoint, AccessPoint),
                            modes: Vec<TravelMode>, direction, corridor: Polyline3,
                            legs: Vec<LegDef>, cost: RulesValue, visibility, status: Active|Inactive,
                            revision }
pub struct TerrainTile    { id, region, owner: TileOwner, bbox: Rect2, resolution_m: u16,
                            cells: Cells, revision }              // immutable per revision
pub struct Feature        { id, kind: FeatureKind, geometry: Geometry3, z_range: (f32, f32),
                            blocks: { movement: bool, los: bool }, state: Intact|Damaged(u8)|Destroyed,
                            room: Option<ObjId>, revision }
pub struct DomainFrame    { domain: DomainId, region, bbox: Rect2, resolution_m: u16,
                            lattice: Hex { size_m } | Square { size_m } | Metric,
                            composited_from: Vec<(TileId, revision)>, terrain_revision }
```

`Cells` is a compact per-tile array: surface class (`u8`), `elev_m` (`i16` cm or `f32`), cover
class (`u8`), passability bitset by medium (`u8`), and a derived `room` index (`u16` into the
tile's room table) so "which room contains this pose on this floor" is one lookup. Indoor floors
are tiles too (one per structure floor), which is what makes a bar-room brawl and a street fight
the same kind of domain.

`Footprint` is `Polygon2 | Cells(TileId, set) | Anchor { point, radius }`; the anchor form is the
authoring default for a brief and is rasterized to cells at build time, so a brief never has to
draw polygons to be valid — it has to when a site is promoted to tactical detail.

### 5.3 What the existing records become

| Today | Under the substrate |
|---|---|
| `SpatialParent::Direct { pose: RulesValue }` | pose is a **metric pose** `{ x, y, z, facing_deg, posture }` in region coordinates; drivers derive lattice cells through `DomainFrame` (D-11a-5). Hex and square drivers keep their maths; they no longer own the coordinate |
| `DomainRecord` | gains `frame: DomainFrame`; `presentation_location` stays as the fallback of the resolver |
| `Obj` for a room | unchanged; geometry is in `RoomGeo` (typed family), never an attribute |
| exit `succ/osucc` | unchanged; a `Portal` references the exit and carries geometry, mode, distance, `dz`, time policy |
| `RouteDef { endpoints: (ObjId, ObjId), legs }` | `Connection`; `LegDef { to, base_s, hazard, interdiction }` is kept as the checkpoint record, `to` becomes a checkpoint on the corridor polyline (an access point at the ends), TacSim's `drive_class`/`propellant`/`alignment_band` move into `cost: RulesValue` |
| `CarrierJourney` | the one journey record; `root` becomes `traveler: Traveler::{Object, Character, Carrier}`; `destination: SpatialParent` stays |
| `route.rs::Journey` | retired; its narration, catch-up and intercept behaviours re-home in the one journey |
| `LocationDef { label, role }` in tacsim packs | a `SiteRec` (or `LocalityRec`) with a footprint in a region whose frame is the solar-system strategic graph's; the corridor's `route` entries are `Connection`s. One atlas for a WoD city and the Mars–Phobos–Ceres corridor, at different scales |
| `mush-faction::Region.zones` | `RegionRef::{Zones, Atlas(NeighborhoodId | LocalityId), None}` |
| `worldgen.theme` on ZMOs | resolves along the atlas hierarchy; zones keep locks and zone exits (D-11a-9) |
| `toy_map`, `set_map_blob` | retired; the `tactics.map` panel keyframe carries a composited window of the domain frame |
| `mush-propagation::Geometry` (range 0/2) | a real metric from poses and `Feature.blocks.los`, which it was written to accept |

### 5.4 Rooms ↔ terrain ↔ domains

- **Coverage** is derived, never declared twice: a domain covers whatever tiles its `bbox`
  intersects; a neighborhood is "covered" by a domain when their footprints intersect. Many-to-many
  falls out. An encounter spanning two neighborhoods is a bbox across their boundary; an encounter
  on an implicit corridor is a bbox around the checkpoint composited from background alone.
- **Compositing**: creating a domain rasterizes every intersecting tile to the domain's
  `resolution_m` (finer tiles downsampled, coarser upsampled, background where none), pins the tile
  revisions in `composited_from`, and snapshots the result under `mod:tactics:map:<domain-id>`
  (doc 06 §9.1 already reserves the key). A domain never edits tiles; **durable consequences write
  `Feature.state`** and the next composite picks them up — which is §5's "ending an encounter or
  regenerating descriptions must not restore them" for free.
- **Entering a domain** from a room: pose = a free cell of `RoomGeo.footprint` on the room's floor
  adjacent to the portal the actor came through, else any free cell, else refuse. **Leaving**: the
  room whose footprint contains the pose on that floor, else the domain's `presentation_location`.
  This is D1's `PresentationResolver`, deterministic and generic.
- **Speech and perception** across a multi-room domain follow the resolver: a participant is in
  the resolved room for `say`/`look`, so doc 11 §5's "presentation room is an interface, not a claim
  of co-presence" becomes literally true.

### 5.5 Authoring the substrate

- **First slice: no drawing.** A brief's site rectangles, room anchors and exit rows are enough:
  `atlas::raster` turns them into footprints (rectangle → cells; anchor → radius), portals (one per
  exit row, geometry at the shared edge or the anchor midpoint), floors (from the structure table)
  and features (walls along footprint edges, doors at portals). The result is a playable, legal
  terrain at a coarse resolution with no LLM in the loop. This is the "narrative-first" hydration
  slice of D-11a-6, and it already yields a bar-level map — a crude one.
- **Second slice: typed features.** The LLM (Stage B, H5) proposes `Feature`s and room footprints
  as typed objects — a counter here, a shuttered window there, a hedge — never cells and never
  exits by name; the validator checks geometry and the rasterizer places them.
- **Third slice: elevation import** (16-bit heightfield or a simple ASCII grid) for regions and
  neighborhoods that want real relief; rural neighborhoods want it first (Mill Creek's stream and
  bridge). Until then the region DEM is flat at the datum.
- **Never**: an image or prose promise standing in for terrain (doc 11 §8 step 3 says this; the
  substrate makes it checkable: a site is `tactical_ready` only when every room has a footprint on
  a tile and every portal has geometry).

### 5.6 Connections and travel (H4, made concrete)

- A `Connection` is region-owned and has two `AccessPoint`s, each `{ neighborhood, room:
  Option<ObjId>, point: Point3 }`. It is `Inactive` while either room is unresolved (an implicit
  neighborhood's side) and activates when both resolve — the fix for H3.
- A journey is `depart(traveler, connection, mode)`: validate authority, departure access and
  movement gates (the `MoveAttempt` hooks veto travelers today; keep that), reserve what the cost
  policy says, stamp `departed` on `WorldIc`, and advance leg by leg on the coarse IC tick with the
  idempotent cursor. Each `LegDef` boundary is a checkpoint; `hazard`/`interdiction` bands are the
  authored exposure the design calls "observation/interception checkpoints"; an interception
  promotes a domain whose `bbox` is centred on the checkpoint's point (C20A, `PromotionTrigger::
  Interception`). Cancel or disconnect leaves the traveler at its last committed checkpoint,
  projected into `in_transit_room` as today.
- Local timed exits (500 m of farm road) are a one-leg journey on the same authority with the
  portal's `distance_m`, `dz_m` and mode; short exits are instantaneous because their policy says
  so, not because they are a different mechanism.
- Modes are ruleset-registered (`foot`, `car`, `transit`, `torch`), each with a cost function over
  `(distance, dz, surface, cost: RulesValue)`; `MobilityMode.nominal_speed_cms` in `mush-tactics`
  is the seed of this.

### 5.7 Bounds

Tiles: cap cells per tile (256×256 at the tile's resolution ≈ 64 k cells ≈ 256 KiB packed) and
tiles per neighborhood; a neighborhood needing more uses a coarser resolution or more tiles.
Domains: `bbox` capped in cells at the requested resolution so a "whole valley" fight is a coarse
map, not a refusal; `MAX_ACTIVE_DOMAINS` stays. Compositing runs off-loop and lands as one bundle
inside the K3 limits. Features per tile and portals per room are capped like every other record.

### 5.8 Storage

`mod:atlas:{region,locality,neighborhood,site,structure,roomgeo,portal,connection,feature}:<id>`
as bounded module documents in one `atlas` typed family under a registered owner and codec (09b);
tiles at `mod:atlas:tile:<id>:<revision>`, immutable; the domain snapshot under `mod:tactics:map:
<domain-id>`; the seed/apply latch at `mod:content:applied:<pack>`. Activation, review decisions
and retirements are `CanonDraft`s (`kind = "atlas.site.activated"` etc.) so the chronicle records
that the bookstore came into being and who approved it.

### 5.9 Scales and ships: the Starfield pattern (premise 6)

Starfield's map is a set of **discrete, bounded cells** — a city, a landing site, a station
interior, an orbit, a system — joined by ship travel, with nothing simulated in between and
encounters possible at the seams (orbit on arrival, a landing site's perimeter). That is doc 10
§2's four scales and 1A's abstracted routes, and it is premise 1 at planetary scale: a neighborhood
*is* a landing zone. The substrate carries it with three additions, none of them a new mechanism.

- **Nested frames.** A `RegionRec` gains `kind: Surface { body } | Orbital { body } | System |
  Interstellar` and an optional `parent: (RegionId, Transform)`. A surface region is a metric frame
  on a body; an orbital or system region is the strategic node graph (its "coordinates" are nodes
  and the `Connection`s between them, with `corridor` geometry optional). The Mars ground theater,
  Phobos orbit and the Ceres–Phobos lane are three regions of three kinds in one atlas; a WoD city
  is one surface region with no parent.
- **Access points that cross frames.** A `Connection` may join access points in different regions:
  a landing pad in a surface neighborhood ↔ an orbital node; a dock ↔ a station interior; a
  station interior ↔ the system graph. Mode `ship` (or `shuttle`, `train`, `car`) is the traveler
  kind allowed on it. "Take off" and "land" are `+travel` on such a connection; the surface
  neighborhood is the arrival room's neighborhood exactly as for a road.
- **Ships are the transport, and they are already the right shape.** A ship is a carrier root
  with anchored interiors, occupants, cargo and stations that move with it (C16A, `CarrierJourney`,
  `SpaceAnchor::MovingObject`). Under D-11a-4 the same record carries a car through a city and a
  hauler through the corridor: the passengers play aboard (C19A), the journey never pauses for a
  scene (6A), an interception promotes the carrier into a domain (C20A), and arrival commits the
  whole subtree at the access point's room. The land-based `+uber` is a carrier journey whose
  interior is a back seat.
- **Landing sites are hydration of implicit space.** Starfield's on-demand surface tile — a bounded
  patch around the landing point with generated terrain and a perimeter — is doc 11 §3's "materialize
  a specific roadside, clearing, or tactical patch" and §5.5's rasterizer applied to background: an
  implicit region point plus a radius yields a coarse tile from the DEM and surface class, a
  perimeter feature, and one arrival room. Persist it only if something consequential happened
  there (doc 11 §3); a landing site players return to is promoted to an implicit neighborhood with
  a stable id, and a built one to authored — the same promotion path as any implicit neighborhood.
- **Scale transitions are ordinary transitions.** Boarding is `embark` into the ship's interior;
  a ship on a landing pad is `Interior`/`Attached` at the pad's site; in orbit it is `Direct` in
  the orbital region's domain or, off-screen, a traveler on a connection. Nothing new is added to
  doc 06 §7; the atlas only says *where* each of those states is on the map.

What this does not import from Starfield: loading-screen fast travel that skips the seams. The
never-skip invariant holds across scales — a checkpoint on a lane, an orbit on arrival, a
perimeter on a landing site are the places encounters are allowed to happen, and a `Connection`
without them is a design choice its author makes visibly in the pack.

## 6. Hydration, as it should be built

Doc 11 §8's lifecycle is sound; this tightens each step to a landed mechanism and names the first
slice.

1. **Find.** `SiteKind` taxonomy + function tags + neighborhood `HydrationPolicy.eligible_kinds`;
   search discoverable sites and open requests by neighborhood and kind; the fit check runs against
   hidden/reserved footprints server-side and answers only "fits / choose another spot / an
   existing site serves this" — never why.
2. **Reserve.** One C8 `ReservationRecord` per resource — the account creation allowance (H6),
   the neighborhood's expansion capacity, and the footprint — under a `ReservationBook`; the request
   id is an `IdempotencyKeyText` and the request document its own record, so retries, second tabs
   and two players asking for the same implied park converge on one request. Abandoned reservations
   expire; an activated site's never do.
3. **Generate.** A pack: `PackKind::Site` with entries for rooms (`RoomGeo`), portals, features,
   NPC profiles and lore (secret-free), hooks, and the tile deltas the rasterizer produced. The LLM
   contributes prose and a typed site plan (H5); everything else is deterministic. Provider failure
   leaves a request in `failed` with its reservations released and the metered spend kept.
4. **Validate and activate.** Validator: geometry (§5.5), topology (reachable from a public access
   point, private ingress/free egress), budget, policy (kinds, reserved land, `Property`), secrets
   (no `Secret` lore in any NPC prompt field), supported definitions. Apply through the latch as one
   `WriteBatch`; activation is deferred to a transition boundary if an exclusive scene or a live
   domain covers the footprint (the timestop and placement gates already veto movement, so the
   rooms simply are not reachable until then). Site record: `origin: Hydrated { request }`,
   `review: PendingReview`, `tactical_ready: bool`.
5. **Persist and queue.** Canon event; `BUILD` ticket with a typed payload naming the request,
   pack digest and neighborhood; reviewers from `NeighborhoodRec.reviewers`, staff fallback. The
   reviewer sees the request, the pack, the validator report, the possible duplicates and what has
   happened there since. Approve records the content revision; `needs_changes` keeps the site live
   and lets the reviewer suspend new entry; retire is an explicit, reference-checked, canon-recorded
   operation and never automatic.

**First slice (D-11a-6):** steps 1–5 with the rasterized coarse terrain of §5.5's first slice and
no LLM topology: the site plan comes from the site-kind template (bar = front room + optional back
room) plus the player's prose for descriptions through the existing `@gen` draft path. That ships
premise 3 end to end, produces a legal if crude tactical footprint, and leaves fine terrain and
typed features as upgrades that reuse the same records.

**Sprawl.** Quotas bound players, not the map. Bound the map with `HydrationPolicy.max_sites`,
`max_added_rooms`, `max_pending_reviews`, and a locality-level cap, all checked at reservation.

## 7. Revised build order

Doc 11 §10's order is close; two changes: records and identity decisions come first as a doc
amendment, and hydration is split from terrain.

| # | Cut | Proves | Depends on |
|---|---|---|---|
| 0 | Doc amendment: §5 records, id scheme, lane, pack kind, travel decision | — | D-11a-1..5 |
| 1 | `mush-content`: the pack loader and latch lifted from `ruleset-tacsim`, with update/retire; the `atlas` family and `@atlas` staff verbs; region + neighborhood pack import; room membership; `+atlas` player view | two neighborhoods and one corridor imported from packs, rooms classified, unchanged pack reapplied is a no-op, a retire with references refused | nothing new |
| 2 | One travel system (H4/§5.6): connections, travelers, `+travel`, local timed exits, checkpoints, interception promotion; `route.rs` retired | the §10 acceptance list for travel: uncrossable river, foot vs road, timed rural path, mid-route restart, checkpoint interception | cut 1 |
| 3 | Narrative-first hydration (§6): reservations, request record, site-kind templates, rasterized coarse terrain, validator, latch apply, review queue, canon events, export | the §10 hydration list minus fine terrain | cuts 1, C8 book |
| 4 | Presence (M6) | the §6 privacy tests | cut 1 |
| 5 | The substrate proper (§5): tiles, elevation, features, floors, domain frames, compositing, metric pose, resolver, `tactics.map` window with SSH fallback | a bar fight and a street fight in composited domains; damage surviving regeneration; an encounter across a boundary | cuts 1–2; coordinates with E7 cut 8's driver |
| 6 | Stage A survey builder (§7) | the §10 survey list | cut 1; can run beside 3–5 |
| 7 | Typed-feature and elevation-import hydration; `tactical_ready` sites | full hydration list | cuts 3, 5 |
| 8 | Faction places (§9) | the §10 faction list | cuts 3, 6; E4b2 |

## 8. Decisions for the owner

| # | Decision | Recommendation |
|---|---|---|
| D-11a-1 | Content identity: pack-scoped `CanonicalId` (`lantern_ward` + `room.bar`) vs a new geographic id type admitting `-` and `/` | **Pack-scoped `CanonicalId`.** One id type, one digest discipline; fixtures are renamed, not re-typed |
| D-11a-2 | Manifest/apply: lift the E7 pack loader + seed latch into `mush-content` and extend to update/retire vs a new registry in worldgen | **Lift and extend** (M1) |
| D-11a-3 | Brief storage: canonical JSON pack as the digested artifact with Markdown+YAML as import/export rendering vs Markdown+YAML as the source of truth | **JSON is truth; Markdown is a view** (M3) |
| D-11a-4 | Travel: one system grown from `CarrierJourney` over atlas connections, retiring `route.rs` vs a third journey type | **One system** (H4) |
| D-11a-5 | Pose: canonical metric pose in region coordinates, drivers derive lattice cells vs driver-owned opaque poses (doc 06 §5.1) | **Metric pose** (§5.3). This is the decision that makes rooms and maps one system; doc 06 §5.1's `RulesValue` pose stays for driver-private posture only |
| D-11a-6 | First hydration slice: narrative-first with rasterized coarse terrain vs terrain-required from day one | **Narrative-first** (§6) |
| D-11a-7 | Creation allowance: per-account counter in the integrity lane via a fail-closed provider; compute as `BudgetScope::Actor` under a `WorldgenSite` feature vs per-character only | **Per-account allowance, per-actor compute** (H6) |
| D-11a-8 | LLM topology: typed site plan under a deterministic validator, template-constrained for hydration vs free topology | **Typed plan, templates for hydration**; record the reversal of "never digs" (H5) |
| D-11a-9 | Theme and grouping: theme resolves along the atlas hierarchy, zones keep locks/exits, `RegionRef::Atlas` vs zones stay the theme carrier | **Atlas carries theme** (M7) |
| D-11a-10 | Naming: keep "Region" for the atlas with `@atlas/*` commands vs rename the tier | **Keep, under `@atlas`**; faction docs say "control region" |
| D-11a-11 | Doc structure: renumber to 14 and split three ways vs keep one doc | **Split** (M11) |
| D-11a-12 | Elevation: region DEM + neighborhood fine tiles + structure floors as one field vs per-room `z` | **One field** (§5.1) |
| D-11a-13 | Scales: nested region kinds (surface, orbital, system, interstellar) with cross-frame connections and ships as carrier travelers vs a separate strategic graph beside the atlas | **One atlas, nested frames** (§5.9); the tacsim corridor pack's locations and routes become atlas entries |

## 9. Amendments owed

To doc 11:

- §1 requirements: add premise 4 (elevation and terrain integrated) and premise 5 (no migration).
- §2/§3: add the records of §5.2, the region kinds and cross-frame connections of §5.9, and the id rule (D-11a-1); state that floors and elevation are
  part of the geographic frame; delete "a room anchor is not an actor's precise combat position"
  in favour of footprints and the resolver.
- §4: replace the "future journey extension" with §5.6; cite `RouteDef`, `LegDef`, `CarrierJourney`,
  `advance_due`, `reanchor`, `PromotionTrigger`; retire the sentence "current `@route` cannot
  traverse roomless geography"; name the player verb.
- §5: replace "the selected driver", "declared resolution" and "terrain revision" with `DomainFrame`,
  `TerrainTile`, `Feature` and the compositing rule; state that no terrain exists at `0bc5e92`.
- §7 Stage B: record the reversal of "never digs"; specify the typed site plan and validator list;
  manifest = pack (`PackKind::Neighborhood`/`Site`); apply = latch with update/retire; three-way
  merge = per-entry revisions plus live-state exclusion.
- §7 durable conversation: name the proposal record and the typed `build.proposal` payload (M4).
- §8: reservations are C8; the request id is an `IdempotencyKeyText`; quota scope per H6;
  activation/retirement are canon events; the first slice per §6; the reviewer checklist.
- §10 table: add the un-cited seams listed at the end of §3; correct the `mush-sim::route` row;
  replace the build order with §7.
- Acceptance scenarios: add §10 of this review.

To the template:

- Ids: `[a-z0-9_.]`, pack-scoped; references as `(pack, entry)`; drop `/`.
- Front matter: `elevation_datum_m`, `dem: none | tiles`, `floors` per structure; `requires:
  [{pack, version, digest?}]` instead of `external_dependencies` strings.
- Geography: connections move to the region pack; the neighborhood declares access points.
- Rooms: add `footprint` (rectangle, cells or anchor+radius) and `floor`; exits gain `mode`,
  `distance_m`, `dz_m`.
- Sites: `kind` from the taxonomy; `property` reference where established.
- NPCs: state that `guarded` is tone, `secret` is the boundary.
- Hydration: per-site `tactical_ready` expectation; NPC tier default.

To the examples:

- Rename ids (`lantern_ward`, `room.east_arrival`, …); fix the M9 overlap; move
  `connection.east-road` into a `redwater_basin` region pack that both briefs `require`; add a
  `floors` row for Courtyard Flats (ground landing, first-floor flat) and an elevation note for Mill
  Creek's bridge and stream banks so the fixtures exercise §5.1.

To the README: the row for this review; the renumbering (M11).

## 10. Acceptance scenarios to add

- A hyphenated or slash-bearing id is refused at pack load, naming the entry.
- Two neighborhoods import in either order; the corridor is `Inactive` until both access points
  exist and `Active` after, with no edit to either pack.
- An unchanged neighborhood pack reapplied creates nothing and changes nothing; a pack that drops
  a room with occupants, exits from elsewhere or a scene anchor is refused with the references named.
- A journey on the one travel system: a mid-route restart resumes at the last checkpoint; a
  checkpoint interception promotes a domain centred on that checkpoint's point; a walking character
  and a carrier use the same record and the same never-skip test.
- A room on floor 1 of a structure projects an entry pose whose `z` equals `ground(anchor) +
  z_offset`; a stair portal has a non-zero `dz` and its time policy is applied to a timed exit.
- A domain whose bbox spans two neighborhoods composites both fine tiles and the background, at one
  resolution, and pins every tile revision it used.
- A destroyed `Feature` survives a description regeneration and appears in the next composite.
- A hydrated site with coarse terrain is playable and `tactical_ready = false`; after typed features
  and footprints land, the same site flips to `true` with the same ids.
- The creation allowance is charged once across two concurrent requests for one implied park;
  the second returns the first's request id; compute spend appears under the requester's actor
  budget and the `WorldgenSite` feature, and no account identifier appears in any journal or canon
  record.
- A `guarded` lore fact reaches the prompt with its framing; a `secret` one never does, from a
  hydrated NPC exactly as from a staff-authored one.
- A ship carrying three players and a mounted craft departs a surface landing pad on a cross-frame
  connection, arrives at an orbital node, is intercepted there, and lands elsewhere: every occupant
  keeps one location throughout, the arrival room is the destination pad's, and no scene aboard
  held the journey.
- A landing at an implicit point yields a bounded coarse tile, a perimeter and one arrival room;
  leaving with nothing consequential persists nothing; a destroyed feature there persists the patch.
- Presence: one arrival and one departure inside a tick window do not change the neighborhood's
  bucket; a private interior contributes nothing; no per-arrival delta is emitted.
