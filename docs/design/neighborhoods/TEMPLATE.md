---
brief_version: 1
id: neighborhood.replace-me
name: Replace me
status: draft
representation: authored
region: region.replace-me
locality: locality.replace-me
setting: urban
coordinate_frame: region.replace-me.surface_m
origin_m: [0, 0, 0]
rotation_degrees: 0
footprint_local_m: [[0, 0], [600, 0], [600, 600], [0, 600]]
room_budget: {target: 12, maximum: 18}
external_dependencies: []
---

# Neighborhood brief: Replace me

This is a proposed authoring convention, not a runnable importer. Coordinates use the named
regional frame, +x east / +y north / +z up; this initial template uses zero rotation. Replace all
placeholders and declare external IDs before treating a brief as build-ready. Short IDs in tables
are local to this neighborhood; cross-neighborhood references use the full neighborhood ID plus
`/` and the local ID. Room budgets include circulation and interiors. Explicit `none` is useful
where a section does not apply. For an implicit neighborhood, use a zero room budget and describe
terrain/routes instead of creating a dummy room. See the [design](../11-neighborhood-geography-and-authoring.md).

The in-game authoring workflow uses this template to propose a design and ask focused survey
questions. Players answer, evaluate feedback, and approve the resulting proposal; the system then
generates this brief for staff consideration. Players do not need to fill out a Markdown file.
Ask only questions relevant to the proposal, preserve settled answers, and make new recommendations
explicit. Substantive generated additions require an author decision before submission.

## Proposal provenance (system-generated)

- Source proposal ID, exact approved revision, author, and author-approval record:
- Template/schema version, applicable policy and permitted exemplar references:
- Accepted design decisions and explicitly deferred staff rulings:
- Compiled brief identity/hash and linked staff submission/revision:
- Prior brief and change summary when responding to requested revisions:

These fields are recorded by the system, not accepted as self-asserted authority from an uploaded
file. Author approval means compile and submit; staff approval and build authorization have their
own records. Keep raw survey history accessible to authorized reviewers without copying it all
into the brief. Import/export preserves provenance without conferring approval on imported claims.

## Intent and scope

- The neighborhood in one sentence:
- Why players come here and where strangers can plausibly meet:
- Urban/suburban/rural pattern and real physical extent:
- What is on-screen; what remains background; what must not be expanded:
- Canon/ruleset constraints and approved exemplar references:
- Unresolved decisions (explicit staff-ruling requests may remain at submission; resolve required
  implementation choices before a build-ready handoff):

## Public look and feel

Describe street/land use, age/materials, maintenance, light, sounds, smells, weather exposure,
day/night differences, and two or three recurring motifs. Include contrasts and human particulars.
Specify prose POV, tense, length, vocabulary to favor/avoid, and details to inspect. Background
people are ambience unless explicitly authored as NPCs; never imply player actions or emotions.

## Geography, neighbors, and routes

Describe boundaries, barriers, road/trail/water networks, elevation where relevant, implicit
neighbors, regional connections, and future expansion anchors. Coordinates below are neighborhood
local meters unless marked regional. Geometry and route distances must be consistent.

| Connection ID | Local endpoint / anchor | Other endpoint | Kind / modes / direction | Route, distance/time policy, access and visibility |
|---|---|---|---|---|
| connection.replace-me | room.replace-me at [x,y,z] | Full external ID | boundary or journey; walk/drive; both/one-way | Corridor, implicit areas crossed, barriers/checkpoints, arrival, clock policy |

List external dependencies (region, locality, corridor, destination, ruleset definitions) using
stable IDs. Say which are pre-existing requirements versus proposed companion content. Missing
external references block activation; the builder must not invent a destination.

## Sites and structures

| Site ID | Name / kind | Anchor or footprint | Public entrance(s) | Purpose / access / room budget |
|---|---|---|---|---|
| site.replace-me | Name; building/park/farm/compound | [x,y,z] | room.replace-me | Reason to visit, public/private areas, required rooms |

For multi-building sites, enumerate structure IDs, functions, entrances, floors, and footprints.
For each site, give distinctive materials, sensory details, services, ownership reference if
established, and facts that differ from neighborhood defaults. Do not invent ownership grants.

## Room intents and local topology

| Room ID | Name | Site / structure or neighborhood circulation | Anchor / floor | Public or restricted | Scene purpose and description requirements |
|---|---|---|---|---|---|
| room.replace-me | Name | site.replace-me or circulation | [x,y,z]; ground | public | What someone can do here; what merits a separate room |

| Exit ID | From | To | Names/aliases in each direction | Access / timing / visibility |
|---|---|---|---|---|
| exit.replace-me | room.a | room.b | east;e / west;w | public, short local walk; or explicit gate/timed path |

Each row declares both directions unless explicitly one-way. Describe hidden/locked routes in
the staff section and specify what public viewers see instead. No disconnected rooms without an
intentional access explanation. State the public gathering room and arrival rooms. List background
facades, inaccessible floors, and scenery that must not become extra rooms.

## NPCs

Repeat for each NPC, with a stable `npc.*` ID:

- Name and role; neighborhood-wide or site-scoped (site ID); tier:
- Initial room, routine/allowed locations, and when they are available:
- Distinct personality, voice, background, wants, fear, and contradiction:
- Relationships to other named NPCs and hooks:
- Public facts, guarded facts and intended sharing conditions, things they do not know:
- Permitted lore-pool references; staff-only facts are separately listed below:
- Ruleset/sheet requirements, abilities/services supported by existing mechanics:
- Fallback lines, bounded behavior, and what requires staff handling:

Routines are authoring intent until supported by a scheduler; do not claim that listing one makes
it execute. Avoid homogeneous NPC personalities inferred solely from a neighborhood stereotype.

## Hooks and local tensions

For each `hook.*` ID: public invitation, relevant NPCs/sites/rooms, stakes, player choices, several
clue/approach paths, possible persistent consequences, and supporting mechanic/definition IDs.
Mark unsupported quests or rewards as staff-run. Do not predetermine success, compel participation,
or create spendable assets from a narrative suggestion.

## Staff-only knowledge and hidden spaces

For each secret: stable ID, truth, who fictionally knows it, where it is stored, who may receive
it in prompts, revelation trigger, and public/guarded facts produced after revelation. Use explicit
tiers, not a general "everyone here knows" paragraph. Current `secret` NPC lore is excluded from
runtime NPC prompts; use approved staff/scripted revelation instead of embedding it in persona.

List hidden entrance geometry, access/discovery requirements, private rooms, and map redactions.
The authoring file is privileged source material, not a player-facing export.

## Play discovery and tactical grounding

- Public gathering places and optional joinable-scene invitations:
- Eligible open-to-play presence; explicit private/hidden exclusions:
- Terrain, cover, elevation, structure extents, doors and vehicle access:
- Exact detail deferred until tactical terrain authoring (label estimates):
- Stable features whose damage or alteration must survive rebuilding:

## Builder acceptance and change ownership

- Required room/site/NPC/hook IDs and budget; public-path and access checks:
- Required geographic alignment and arrival points:
- Prose quality and distinctive NPC checks:
- Secret/map/prompt filtering checks:
- Supported versus staff-run mechanics:
- Source-owned fields (initial topology, descriptions, initial authored profile/lore):
- Live state to preserve (memories, relationships, inventory, ownership, scenes, damage):
- Expected result of a repeated build: no duplicate objects; unchanged manifest is a no-op.
- No silent deletion, unresolved references, placeholder prose, or speculative extra rooms.

## Player-requested site hydration

Specify whether player hydration is enabled. The initial room budget is not permission to invent
extra rooms; the policy here explicitly allocates expansion beyond the initial manifest.

- Eligible site kinds and the scene needs they serve:
- Implied sites available for hydration, or allowed footprints and connection anchors:
- Excluded/reserved land, established facts, property/access constraints, and duplicate criteria:
- Maximum added sites, total added rooms, and concurrently pending reviews for this neighborhood:
- Per-site rooms, footprint, tactical terrain resolution/size, NPCs, and supported services:
- Account-level quota-policy and compute-budget references (defined by the game):
- Required package: stable IDs, descriptions/details, connected rooms/exits, working tactical
  terrain and entrance mappings, permitted NPC/lore/hooks, atlas/presence integration:
- Activation policy: complete validated sites may be active with `pending_review` under the
  neighborhood's delegated policy, without waiting for a human review:
- Review responsibility and handling of corrections or occupied duplicates:
- Durable site-addition registry and export convention for Git reconciliation:

Activated sites persist for future scenes. No inactivity, quota-renewal, or review-age expiry.
Failures release creation reservations but retain metered generation costs. Rebuilding the base
brief preserves registered additions and live state. Tactical detail may be deferred for an initial
narrative-only neighborhood build, but a site advertised as fully hydrated requires actual validated
tactical terrain, not a placeholder.

## Faction place proposal

Complete this section for faction bases, Elysia, headquarters, and other facilities requesting
faction rights or material capabilities. Use `not applicable` for ordinary public sites. A single
building uses a site-scoped version of this brief with its parent neighborhood identified; only a
real neighborhood proposal needs the full locality footprint. This section supplements the room,
NPC, geography, and style sections, without requiring filler content at a larger geographic scale.

- Stable proposal ID/revision; faction/organization IDs; submitting actor and authority/scope:
- Scope (site or neighborhood); parent geography; existing holding, acquisition, renovation, or
  new construction; evidence of established rights:
- Proposed owner, operator, access administrator, public access, and faction-private areas:
- Contribution to faction identity and opportunities for other players to engage:

| Resource / obligation | Amount and unit / capacity | Source / payer / entitlement ID | Evidence status | One-time or recurring / timing |
|---|---|---|---|---|
| Replace with acquisition, rent, construction, staffing, etc. | Explicit quantity or unresolved | Stable reference or unresolved | authoritative / quote / estimate / unknown | When needed, cadence, and dependency |

| Capability | Extent / limit | Supported definition or staff-run policy | Inputs / upkeep | Weakness / loss-of-support consequence |
|---|---|---|---|---|
| Replace with security, NPCs, storage, services, etc. | Number, strength, capacity, access | Versioned ID or explicit unsupported status | Resource references | Operational effect; preserve physical place/history |

- Separate social conventions, descriptive details, and executable mechanics. Specify NPC roles,
  sheets, allegiance/authority, coverage, and staffing; no implied bonuses from impressive prose.
- Construction/delivery phases, clocks, prerequisites, and any separately usable completed stage:
- Resource uncertainties, power/style policy references, and approved comparison examples:
- Public, faction-private, and staff-only source/report boundaries:
- Requested exceptions or staff grants, with explicit extent and justification (`none` if absent):
- Staff decision maker/policy; separate faction spending approvals and execution authority:

The author's in-game approval compiles this brief and submits it with topology/placement previews,
proposed NPC profiles/capabilities, resource schedules, deterministic checks, and applicable LLM
findings, uncertainties, and suggested revisions. Staff approve, request targeted survey changes,
or decline that concrete revision. Full room/terrain/manifest generation follows the approved brief;
any material divergence returns to review. Staff content approval precedes live activation; ordinary
`pending_review` hydration does not grant faction capabilities. Conditional approval waits for its
conditions. Revalidate funds, rights, authority, and approved revisions at execution. Preserve
proposal, decisions, generated baseline, and all subsequent live state.
