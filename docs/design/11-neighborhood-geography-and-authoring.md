# Neighborhood geography, movement, and authoring

**Design draft — 2026-09-07; owner rulings R1–R4 added 2026-09-09 (§11).** Records the owner's
neighborhood-first direction and proposes vocabulary and contracts for implementation. Where §11
conflicts with the text above it, §11 governs; the [review](11a-neighborhood-geography-review.md)
records what the tree holds and what the rulings decided. The requirements below come from that conversation;
the particular names, records, defaults, and build sequence are recommendations, not previously
ratified decisions. This document adds no runtime features.

Companions: [spatial domains](06-object-behaviors-spatial-domains.md),
[NPC mobility](07-npc-control-and-tactical-ai.md),
[economy and organizations](08-economy-organizations-ownership.md),
[TacSim foundations](10-tacsim-foundations.md),
and [current worldgen](../help/worldgen.md). The [brief template](neighborhoods/TEMPLATE.md) and
[urban](neighborhoods/examples/lantern-ward.md) / [rural](neighborhoods/examples/mill-creek.md)
examples are design inputs for a future builder, not executable import files.

## 1. Owner requirements

- No world-scale contiguous grid of MUSH rooms. A room must not stand for several distant places
  merely to bridge a gap in the grid.
- Concentrate rooms, descriptions, NPCs, and opportunities for scenes into coherent, connected
  neighborhoods. This must work for urban, suburban, and rural settings.
- Give neighborhoods consistent positions on a city/regional map. Unbuilt neighborhoods and the
  intervening geography still exist; they need no permanent room graph.
- Relate tactical terrain to this same geography so engagements can cover neighborhood-scale areas.
- Let players move locally and between neighborhoods without requiring filler-room traversal.
- Support neighborhood-level discovery of opportunities to play without exposing exact locations
  or secrets.
- Author a neighborhood with an LLM through in-game commands, using a template and accepted
  examples. Present survey questions as a proposed design; collect answers and feedback; explicitly
  evaluate them to produce focused follow-up surveys. Author approval generates the actual brief
  for staff consideration. Preserve Git export and a separate build stage for rooms, buildings,
  NPCs, personalities, and hooks. This refines the original external-chat/manual-Git workflow.
- Allow players with a small quota to hydrate a fitting business or location on demand, including
  its rooms, descriptions, tactical map, and neighborhood integration. Successful sites persist
  for future scenes and may be available with `pending_review` until a human builder reviews fit
  and duplication. The lifecycle and limits in §8 are proposed implementation details.
- Let faction leaders author proposals for bases, Elysia, headquarters, and other faction places,
  using the brief workflow. An LLM reviewer assesses the proposal before a staff decision maker
  confirms its resource plan, game style, and power level. §9 proposes the review contract.

The governing design principle is **continuous geography, selectively authored room maps**.

## 2. Canonical vocabulary (proposed)

| Term | Meaning | Examples |
|---|---|---|
| **Region** | A geographic frame containing localities, countryside, and transport networks. | Metropolitan region, river basin, lunar surface region. |
| **Locality** | A named settlement or surrounding area organizing neighborhoods. A city is a locality kind, not a required urban assumption. | City, town, rural valley, unincorporated area, surface colony. |
| **Neighborhood** | The principal unit of local worldbuilding, navigation, shared character, and play discovery. Has a geographic footprint and an authored or implicit representation. | Entertainment quarter, suburban crossroads, farming community. |
| **Site** | A distinct destination or property within a neighborhood. May have grounds and multiple structures. | Bar, apartment complex, farm, park, factory, cemetery. |
| **Structure** | An optional building or constructed part of a site; useful when the distinction matters. | Farmhouse and barn on one farm; separate blocks of a campus. |
| **Room** | One coherent place in which MUSH participants can share a scene, with named exits. | Barroom, courtyard, short street segment, clearing. |

Default browsing hierarchy: **region → locality → neighborhood → site → room**.
Outdoor circulation rooms can belong directly to a neighborhood. Sites need no structure; a
single-building site needs no extra structure record. Lots/parcels describe land division when
ownership or layout needs it, not a mandatory room-map level. A district may group neighborhoods
for navigation or administration without becoming another required tier.

This is geographic classification, not recursive MUSH containment. Ordinary rooms remain
`ObjKind::Room` with their existing contents/exits semantics; a building is not a room containing
other rooms. A narrative **scene** is an activity with participants and time, not a zoom level.
A **zone** remains the existing theme/delegation mechanism. A **domain** remains a simulation
boundary. Neither is a synonym for neighborhood.

Map zoom and simulation scale are different axes:

| View | Shows | Does not imply |
|---|---|---|
| Regional/locality atlas | Neighborhood footprints, landmarks, roads, terrain, travel connections. | Permanent rooms or active simulation everywhere. |
| Neighborhood map | Sites, local circulation, public access points, known connections. | Every parcel must be enterable or every building must have interiors. |
| Site/room map | Authored floors, grounds, rooms, doors, and meaningful local paths. | Uniform room size or a square grid. |
| Tactical overlay | Metric terrain, elevations, cover, participants, and entrances at a selected resolution. | A hex per room, or exactly one domain per neighborhood. |

Doc 10's interior/tactical/operational/strategic scales remain valid. A neighborhood is geography
that several of those scales can present, not a replacement fifth simulation scale.

## 3. Geography and room topology

Maintain three linked descriptions of the same world:

1. **Geography:** footprints, coordinates, terrain, sites, transport corridors, and barriers.
2. **Traversal:** local room exits plus regional connections through implicit geography.
3. **Presentation:** the atlas, room prose, and tactical terrain, each filtered for its viewer.

Geographic proximity is not traversability. Adjacent footprints across a river require a bridge,
boat crossing, or another supported mode. A straight line on a map is not a road. Conversely,
two authored room graphs need not touch for a valid journey to connect them.

### Coordinates and identity

- Use stable, namespaced content IDs for regions, localities, neighborhoods, sites, structures,
  rooms, connections, NPCs, and hooks. Display names and runtime dbrefs can change independently.
- Each region declares its coordinate frame, physical units, origin/datum, and axes. A surface
  region should begin with a local metric frame: +x east, +y north, +z up. An optional geographic
  reference can align it to a real map. Do not infer meters from an image's pixels.
- A neighborhood has a footprint in that frame, a local origin/orientation, and a transform into
  region coordinates. The initial authoring format can use axis-aligned rectangles; irregular
  footprints can be added without changing identities. No requirement to tessellate the region.
- Sites and structures have local anchors and, where useful, footprints. Exterior rooms have a
  representative anchor or area; interiors identify their structure/floor and entrance mapping.
  Floors may share x/y coordinates. A room anchor is not an actor's precise combat position.
- Bare edges in a room graph do not determine distance. Meaningful local distances and access
  points must agree with the physical layout; a short exit cannot secretly cross a city.
- Every fixed room has one primary neighborhood classification, including private interiors.
  Cross-boundary sites can reference multiple neighborhoods, with explicit room membership.
  Mobile interiors derive current geography through their host; their home port is not location.
- Boundary ownership is explicit at shared access points; overlapping thematic, jurisdictional,
  or faction overlays must not create duplicate physical membership.

### Authored and implicit neighborhoods

`authored` means a neighborhood has a maintained playable room graph. `implicit` means it exists
on the atlas with enough terrain, transport, atmosphere, and jurisdiction data to support travel
and later expansion, but has no permanent room graph. These labels describe detail, not whether
the place is secret, accessible, inhabited, or currently busy. Visibility is a separate property.

Unnamed background land can remain regional terrain. There is no obligation to invent a named
neighborhood for every gap. Named implicit neighborhoods are useful future build targets.
Promoting one to authored preserves its identity, frame, footprint, and external connections.

An implicit neighborhood can host a bounded encounter when play calls for it. Materialize a
specific roadside, clearing, or tactical patch; never describe the whole implicit area as one
shared social room. Persist consequences and any still-referenced encounter space before cleanup.

### Neighborhood density

Connected means players can understand and walk the meaningful local paths, including locked or
conditional paths where the fiction calls for them. It does not mean every place is publicly
accessible, nor that every few meters needs another room.

Each brief declares an initial room budget and a small public circulation backbone. Every room
should justify itself with a distinct scene purpose, encounter, decision, or necessary connection.
Describe background buildings and acreage as scenery until they need play spaces. An initial
target of roughly 8–20 rooms is a useful authoring prompt, not a universal cap or a physical size.

| Setting | Compact playable core | Surrounding geography |
|---|---|---|
| Urban | A few streets, a bar, repair shop, courtyard, and selected interiors. | Other blocks and inaccessible floors remain background. |
| Suburban | Crossroads, shopping strip, pocket park, selected homes, and connecting paths. | Housing tracts, gardens, parking areas, and arterial roads retain their scale. |
| Rural | Farm gate, common, store, bridge, farmhouse, barn, and woodland meeting place. | Fields and long roads can span kilometers with explicit local travel time. |

Compactness measures the amount of navigation required to reach play. Rural distances remain real:
a named path between two precise locations can take time without acquiring ten filler rooms.

## 4. Movement within and between neighborhoods

Distinguish three transitions:

- **Local exit:** an ordinary door, street connection, staircase, or path between authored rooms.
  It carries access rules and any meaningful distance/time. Short movement stays straightforward;
  a long rural path invokes timed movement through the same travel authority.
- **Boundary connection:** an explicit crossing between genuinely adjacent authored locations,
  such as continuing across a shared street boundary. It may feel like an ordinary exit while
  updating neighborhood membership. One-way access must be deliberate.
- **Journey connection:** a route from a departure access point to an arrival access point through
  geographically located off-screen space. No room chain is required between them.

Each connection identifies stable endpoints, allowed travel modes, directionality, route geometry
or corridor, distance/time policy, access/discovery rules, and relevant crossings/exposure points.
Several connections may link the same pair of neighborhoods: footbridge, road, rail, and hidden
tunnel can differ in risk, knowledge, duration, and availability. Public and secret endpoints
must not be revealed merely because the atlas knows their physical coordinates.

The player experience is: leave a local venue → reach a suitable departure point → choose a known
destination and mode → receive a route/ETA → travel → arrive at a specific entry room. Destination
selection can guide local walking to the departure point; it cannot bypass a locked building exit.
Players need not type an exit command for every unremarkable stretch of road. Public transit can
offer a shared onboard scene; private cars use their own interiors; foot travel can have a scoped
journey presentation. Unrelated travelers are not globally co-present in one generic transit room.

The runtime contract for the future journey extension:

1. Validate actor authority, departure access, mode, route, costs, and movement/scene restrictions.
   Keep the character in one authoritative movement state; travel must not escape combat or locks.
2. Reserve what is necessary and record the route revision, departure time, progress, ETA, and
   ordered observation/interception checkpoints. Clock and compression policy are explicit; room
   count never determines travel time. Cross-city travel has nonzero duration under the existing
   mobility design; exact WoD pacing remains a tuning decision.
3. Advance coarsely until a relevant boundary or encounter. Never skip a registered observation or
   interception opportunity; promotion to an active space happens before resolving that crossing.
   Only authorized observations reveal travelers, passengers, or routes.
4. Revalidate changing access and destination conditions before entry. A closure produces a lawful
   hold, reroute, or nearby stopping point. Canceling or disconnecting does not teleport the actor
   back to departure or forward to arrival; preserve current progress and apply recovery policy.
5. Commit arrival, membership, location projection, and durable state together. Restart recovery
   resumes from durable progress and resolved checkpoint identities, without duplicate arrivals.

Journey time, narrative time, and tactical time follow the existing clock contracts. Crossing
between a paused scene and live travel needs the existing scene transition rules, not an ad hoc
neighborhood exception. Regional travel supplies geography for Doc 07's off-screen mobility model.

## 5. Tactical maps use the same places

A tactical domain references a geographic footprint, a transform between driver coordinates and
region coordinates, terrain revision, and a declared resolution. It may cover part of a
neighborhood, one neighborhood, several neighbors, or an implicit corridor. Neighborhood and
domain IDs have a many-to-many coverage relationship. Neighborhood creation does not start a tick
loop, and a large rural footprint does not require one enormous tactical map.

Rooms, building footprints, walls, gates, roads, and terrain use shared anchors. Entering combat
chooses a legal precise pose from the actor's existing location and access point; a room centroid
must not place everyone together, outside their building, or across a locked door. Indoor movement,
egress, boarding, and return to narrative rooms use validated simulation transitions.

Keep the existing single authoritative placement graph. Geographic membership is a derived
classification of location/pose, not a second physical parent. One participant belongs to at most
one effective domain. Overlapping engagement requests must reuse or coordinate with existing
domain authority; they must not simulate the same actor or destructible structure twice.

The initial sim implementation projects a domain's direct participants into one presentation room.
That presentation room is an interface, not a claim that all occupants are geographically or
perceptually co-present. A geographically resolved room presentation needs the generic resolver
extension anticipated by Doc 06 D1, with visibility and speech rules audited before rollout.
Neighborhood presence for tactical participants must come from geographic placement, not simply
counting occupants of that presentation room.

Durable damage, barricades, demolished entrances, and other consequences attach to stable geography
and world objects. Ending an encounter or regenerating descriptions must not restore them. Implicit
terrain that becomes contested or permanently altered needs a retained state record.

## 6. Discovering play without revealing secrets

Offer a neighborhood-level **open-to-play presence** projection. Its purpose is to help someone
choose an area in which to seek a scene; it is not an intelligence service or a complete census.

Proposed default: characters explicitly offering public play contribute; private/hidden scenes,
concealed activity, staff observation, disconnected actors, NPCs, and transit through the area do
not. Count one active character per contribution, not browser sessions. The exact consent and
idle thresholds should be decided before implementation.

Exact real-time population counts can reveal a hidden meeting through arrival/departure changes,
even without names. Prefer a cached, coarsened signal such as "some open play" / "busy"; small
populations need suppression or explicit consent. Do not advertise this as mathematically anonymous:
outside knowledge can still narrow an aggregate. Display "no advertised activity", never assert
"empty", when nothing is eligible to appear.

If exact counts are offered, label them **characters advertising open play**, never total players
present. Players may separately advertise a public venue or joinable scene by consent. An aggregate
does not make private rooms discoverable or grant entry to any scene.

The server sends only the permitted projection. No hidden names, exact room IDs, secret sites,
faction filters, or raw occupancy arrays should reach the public feed. Avoid immediate per-arrival
deltas and per-viewer population filters that expose differences. Broader totals must use the same
eligible population and disclosure policy; do not expose private activity by subtracting public
neighborhood counts from an all-character locality total. Cache, timing, suppression, and boundary
cases need explicit privacy tests.

## 7. Neighborhood briefs and the two-stage builder

Use **Markdown with small YAML front matter** for the author-controlled brief *(superseded by ruling
R2, §11: the brief and the manifest are JSON packs; Markdown is at most a rendering)*. Keep stable IDs,
geographic anchors, external references, status, and budgets structured; use prose and tables for
atmosphere, local history, sites, room intent, characters, relationships, and hooks. The template is
an authoring convention pending a real schema/parser. Unknown required fields must not silently
default when that parser arrives.

### Stage A: in-game proposal and survey rounds

The primary player workflow is a saved in-game design conversation. The **proposal** is the evolving
design and its decisions; a **survey round** presents focused questions and concrete recommendations;
the **brief** is compiled after the author approves the proposal. The later **build manifest** is
the exact realization of that brief. A chat transcript alone is not an authoritative decision record.

Start with the player's idea and the applicable template, game style, permitted geography, and
approved examples. The LLM proposes a coherent first design: purpose, scale, look and feel, places
to meet, key sites/NPCs, and tensions. Present a small survey with recommended choices and
alternatives tied to that design. Recommendations remain unaccepted until chosen; silence and
omitted answers are not acceptance. Free-form answers, corrections, and broader feedback remain
available throughout.

An illustrative first round for an arts-hall Elysium might propose:

- **Public role:** a working gallery with faction meetings after closing. Keep that public role,
  make it a members' club, or describe another arrangement.
- **Footprint:** a small site with gallery, hall, and offices inside the existing neighborhood.
  Confirm that scope or explain the need for a larger campus.
- **Host:** a curator balancing the arts program against faction demands. Keep, revise, or replace
  that character concept.
- **Resources:** identify whether this is an established holding or a new lease, and the proposed
  payer; unknown costs remain explicit questions for later evaluation.

The player answers or edits several items, then invokes **evaluate**. Answers save immediately;
evaluation is an explicit bounded LLM request, not a call on every keystroke. It reconciles feedback
with the proposal, runs applicable deterministic checks, and returns a visible revision summary
plus only the follow-up questions those decisions create. An after-hours gallery may need questions
about opening hours, private access, staffing, and rent. A public pocket park should not receive
an unrelated corporate-security questionnaire.

Each question has a stable ID, a reason it matters, suggested options where useful, and links to
the proposal fields it affects. Preserve settled answers and revisit them only when a change
invalidates them, explaining the dependency. New model suggestions remain proposed; accepting one
answer must not accept adjacent recommendations. Players can explicitly accept displayed defaults
for a named round, request alternatives, or revise an earlier answer. The system can propose
validated coordinates and known references instead of requiring players to invent technical IDs.

The loop ends with an **approval-ready proposal**: a readable consolidated design, accepted
decisions, applicable resource/capability schedule, visible changes, and matters explicitly left
for staff ruling. Resolve questions needed to establish the author's intent; requests for staff
exceptions or uncertain quotes may remain clearly labeled. The author approves that revision or
continues editing. Avoid endless clarification of optional details with expressly accepted defaults.

### Proposed command experience

These commands illustrate the future interface; none is implemented by this document. SSH and web
use the same saved proposal and operations; web survey controls are an alternate presentation.

| Proposed command | Effect |
|---|---|
| `+build/start <scope>=<idea>` | Begin a neighborhood or site proposal and request its initial design/survey. Resolve missing context in the survey. |
| `+build/show <id>` | Show the design, revision, answers, open questions, and job status. |
| `+build/answer <id>/<question>=<response>` | Save a choice or free-form answer without triggering generation. |
| `+build/feedback <id>=<text>` | Save broader corrections or concerns for the next evaluation. |
| `+build/evaluate <id>` | Evaluate saved feedback and return a revised proposal, change summary, and targeted next survey or approval-ready result. |
| `+build/approve <id>=<revision>` | Author approval: freeze the reviewed proposal, generate its brief, and submit it for staff consideration. Grants no build or spending authority. |
| `+build/withdraw <id>` | Withdraw the draft or current submission while retaining history and existing world sites. |

The approve display states its consequence: **generate this design's brief and send it to staff**.
Staff authorization uses the separate review workflow (§9 for faction places). Ordinary hydration
retains §8's configured delegation: a bounded park request can use a shorter survey, and choosing
this interface does not silently remove ordinary sites' ability to activate pending review.

### Compile the approved proposal and hand it to staff

Author approval snapshots the proposal revision, answers, accepted recommendations, constraints,
exemplars, and relevant policy/world references. Generate the Markdown/YAML brief from those facts,
including circulation, site and room intents, individual NPCs and knowledge bounds, hooks, secrets,
and expansion policy. Compile a complete artifact; staff should not need to reconstruct a design
from chat messages. The examples here are fictional fixtures, not adopted campaign canon.

Compilation may organize accepted material and write bounded prose; it cannot silently add rooms,
abilities, wealth, NPC power, plot truths, or other substantive commitments. Validate structured
fields against the approved proposal and check prose against its constraints. If synthesis needs
a substantive new choice, or a changed dependency invalidates the design, return the difference
for an author decision instead of submitting an unapproved interpretation. Faithful compilation
needs no additional acceptance of every paragraph.

The durable submission contains the actual brief, source proposal/revision, author approval,
check results, applicable LLM findings, and explicit staff questions. Link a typed staff-queue
payload to the exact artifact/revision; retries must create one submission, not duplicate tickets.
Store first, then offer Git export. Players need neither repository access nor manual file editing.
Import/export remains available to staff and external authors under the same validation rules.

Staff-requested changes become another focused survey on the same proposal history. The author
answers, evaluates, and approves a new revision, producing a replacement brief and diff. A pending
ticket must never silently switch its decision target; supersede the old revision and retain its
history. Existing jobs cannot reopen terminal payload tickets, so a revision after a terminal
decision uses a new linked ticket. Setting a job's status alone never authorizes a build.

### Durable conversation and async behavior

Persist proposal identity, author/collaborator access, scope, revisions, questions, answers,
feedback, accepted decisions, findings, approval snapshots, artifacts, and linked jobs. Raw chat
is supporting history with configured retention, not the only decision store. Players resume after
disconnect or restart. Access remains limited to authorized authors and staff; faction membership
alone need not expose every faction proposal.

Evaluation and compilation use existing LLM quotas, budgets, audit, and kill switches. Each request
captures its source revision and a durable idempotency key. Concurrent edits and late LLM results
cannot overwrite newer answers or approve stale content. Mark stale results and allow reevaluation;
a timeout or budget failure preserves work in a resumable state. Only complete valid compilation
enters staff review; a token stream is not a submitted brief.

### Stage B: realize the specification

After the relevant staff approval or delegated activation policy, the builder consumes a pinned
brief revision and allowed dependencies and produces a separate,
typed **build manifest**: exact room/exit topology, object identities, descriptions/details, NPC
profiles and lore, supported sheets/behaviors, and hook bindings. An LLM may propose topology in
this new planning stage. A deterministic validator and normal server mutation paths apply it.
The existing `@gen` description generator remains restricted to already established topology
*(ruling R1, §11: that sentence describes the shipped command only; the builder digs)*.

The manifest/build report must expose:

- Source commit/content hash, schema version, generator/template version, relevant generation
  configuration, and dependency revisions. Archive accepted output: rerunning an LLM is not a
  reproducible rebuild; reapplying the same accepted manifest should be.
- Stable content-ID → runtime-object mappings and an exact create/update/retire diff. References
  resolve by ID, never fuzzy display-name matching. Reapplying an unchanged manifest creates no
  duplicates and makes no changes.
- Room/exit graph preview, geography overlay, generated prose, NPC profiles, and knowledge/hook
  assignments. Validate bounds, budgets, physical connections, reachability, access, and direction
  consistency. Flag intentionally private, disconnected, or one-way content for explicit treatment.
- Generation progress and resumable checkpoints. A provider failure leaves a draft with unresolved
  work, not a publicly reachable half-built neighborhood.
- Authored versus live state ownership. A three-way comparison of previous manifest, new manifest,
  and current world protects staff edits. NPC memory, inventory, earned relationships, scene/canon
  history, ownership, and tactical damage are live state, never rewritten from the brief.

Build in staging, validate the complete patch, and activate through a bounded server transaction
or a staged activation record. Expected revisions reject stale updates. Retirement must check
occupants, external exits, scenes, ownership/custody, and other references; removing an ID from
Markdown is not permission to destroy a live object. An undo operation must account for subsequent
play instead of overwriting it.

A configured workflow can authorize building the reviewed specification into its target without
requiring an acceptance click for every room. Generation, validation, diff review, and activation
remain distinct steps; target permissions and publish policy govern activation. This document
does not change current `@gen/accept` behavior or authorize any deployment. Player-requested site
hydration (§8) uses this same builder with neighborhood-authored expansion policy.

### Knowledge and hooks

Separate public descriptions, guarded knowledge, and staff-only secrets in both brief and manifest.
A privileged authoring pass may use approved plot secrets; public-description and runtime NPC
prompts receive only their permitted subset. Never pass the whole source brief to every NPC.
Current NPC `secret` lore is staff-only and excluded from NPC prompts; do not hide plot answers in
`persona`, `background`, freeform goals, or fallback lines to bypass that policy. Specify who knows
a fact, who may share it, and how a revelation becomes available through a staff/scripted event.

Hooks describe invitations, stakes, actors, locations, and several approaches or clues without
dictating a player's response. A hook is not automatically an executable quest. Bind supported
scene/clock/contract/script mechanisms explicitly; mark unsupported mechanics as staff-run. Do not
invent working command names, grant abilities, or mint economic rewards from prose.

## 8. Player-requested site hydration

**Hydration** makes a previously implicit location into a persistent, fully playable **site**.
For example, players need a park during a scene, the neighborhood has plausible background space
for one, and an eligible player spends a small creation allowance to bring that park on-screen.
A business or other local destination follows the same path. Hydration adds detail to established
geography; it does not mean a park was physically constructed that instant. Actual construction
remains a separate in-character process.

The owner's direction is persistence and human review after creation: once successfully hydrated,
the place can host this scene and future scenes. There is no automatic teardown after departure,
inactivity, quota renewal, or a long review wait. Proposed player-facing labels are **Create a
local place** or **Bring a place on-screen**; "hydrate" can remain the builder term. These are
interface concepts, not implemented commands.

### Neighborhood-authored expansion policy

Add a hydration policy to the neighborhood brief: eligible site kinds, background locations or
available footprints, connection points, style/canon constraints, per-site room/terrain/NPC limits,
and a total expansion allowance. Authors can reserve a specific implied park or let the builder
choose an unclaimed fitting location. Every conceivable business need not be pre-enumerated.

An author can disallow expansion in a location or reserve it for later content. Hydration cannot
replace an established site, contradict a description, consume someone else's property, or create
a shortcut through an inaccessible private place. Players contribute creative intent; normal
builder permissions and world rules govern the result. Creating a shop does not grant its requester
ownership, money, stock, or privileged access. Working services use supported definitions and
their normal authorization.

Faction facilities and requests for material advantages follow §9's staff approval path before
activation. They cannot acquire those capabilities through §8's ordinary pending-review allowance.
Routing follows the requested effects and rights, even if the submission calls itself a public
park, cafe, or cosmetic update.

A park might need an entrance garden and one sheltered meeting space, with remaining grounds
represented by terrain and description. A distinct second park can be appropriate; duplicating
the same park under a new name is a different case. Small player quotas alone cannot prevent
long-term room sprawl, so neighborhood growth and pending-review capacity also need bounds.

### Request, build, and immediate use

1. **Find a place.** Search discoverable sites and pending requests by neighborhood, function,
   location, and identity. Offer a suitable existing park before proposing a new one. An explicit
   choice to create a distinct site can proceed within the authored policy. Authoritative fit
   checks also account for hidden/reserved geography without returning private names, coordinates,
   or reasons. Placement responses must not become a secret-site lookup tool.
2. **Reserve the request.** Validate eligibility and atomically reserve an account-level creation
   allowance, neighborhood expansion capacity, and a footprint/connection point. Durable request
   IDs and revision checks prevent retries, multiple sessions, or simultaneous requests for one
   implied site from creating duplicates. Reuse visible in-progress requests when appropriate.
   Abandoned job reservations can expire under recovery rules; successfully activated sites cannot.
3. **Generate the complete package.** The manifest pipeline creates stable site/room/exit IDs,
   descriptions/details, a physically valid tactical terrain asset, entrance and room-to-terrain
   mappings, and permitted NPCs, public lore, and scene hooks where appropriate. A park can be
   complete without its own NPC. Connect it to the walking graph, atlas, theme resolution,
   knowledge/discovery policy, and presence classification. Exact terrain, cover, elevations,
   and traversable boundaries validate against the selected driver; an image or prose promise
   is not a working tactical map. Persist terrain without starting an idle combat simulation.
4. **Validate and activate.** Check geometry, topology, access, budgets, supported definitions,
   secrets, and current world revisions. All required assets must be ready before the site becomes
   reachable. Neighborhood policy can authorize activation as **active + pending_review**, so the
   human review queue does not prevent play. Report generation progress and offer an existing
   location while work runs; do not claim the park exists before activation. New geography must
   not rewrite an ongoing combat map or bypass scene/movement gates; defer activation to an
   appropriate transition boundary when necessary.
5. **Persist and queue review.** Commit the site, connections, generated assets, content registry,
   provenance, quota charge, and review entry with one idempotent activation. Players then reach
   it through ordinary movement; requesting it teleports nobody. Later scenes reuse the same
   objects, terrain, NPCs, and accumulated consequences.

Quota amounts and renewal periods remain tunable. Proposed accounting is one creation charge per
successfully activated site, reserved during generation; provider/validation failure releases the
creation reservation. Actual LLM usage remains metered even when a request fails. Technical retries
reuse the request ID and budget, with bounded attempts. Returning an existing site consumes no
creation allowance. Account-level limits prevent alts or tabs multiplying the quota *(superseded by
ruling R3, §11: the limit is character-scoped and counts outstanding pending reviews, not
creations)*; compute and neighborhood limits address different resources. Inactivity or account
deletion leaves sites intact.

### Review is independent of existence

Build state (`queued`, `generating`, `failed`, `active`) and review state (`pending_review`,
`approved`, `needs_changes`) are separate fields. **Pending review means playable and durable once
active.** It does not mean a temporary scene instance or disposable draft. Review metadata belongs
on the site/build record; narrative output need not interrupt scenes with implementation labels.
Builders see the request, generated package, policy/source revisions, placement, possible
duplicates, and subsequent usage or edits.

A reviewer can approve, make a scoped correction, or request changes while preserving live state.
Approval records the authored content revision. A later material generated change returns that
revision to review; routine play changes such as memories or damage do not require aesthetic review.

For a true duplicate, compare both sites and their history before consolidation. Preserve stable
references through aliases/redirects or an explicit migration. Distinct NPC memories, inventory,
ownership, and history cannot simply be collapsed by display name. Occupied rooms and encounters
must reach a safe transition before structural changes. Correcting prose must not erase scenes
that happened there. Retirement, if necessary, is an explicit reviewed lifecycle operation with
reference checks and a durable record; no automatic deletion of unreviewed or little-used sites.
A reviewer can suspend new entry during a serious repair while preserving the site and lawful
egress/recovery for existing occupants.

### Reconcile additions with Git-authored neighborhoods

Persist the request, effective source/policy snapshot, manifest, terrain, content IDs, and review
changes in the world's durable content registry immediately. Git export is not a prerequisite for
surviving a restart or for another player using the site.

Export an addition as a site brief plus accepted manifest/assets and a neighborhood-index patch.
Reviewed content can be checked into Git with the same IDs. The base neighborhood brief and its
registered additions compose the next build input: rebuilding an older brief preserves later
hydrated sites even if they have not been exported. Reimporting an exported addition reconciles
the existing site. Source/world revision conflicts require a merge; absence from an older file
never implies retirement.

The examples include bounded park hydration policies. Their original room counts remain the initial
build; hydration explicitly consumes spare expansion capacity. Full hydration depends on both the
manifest builder and working tactical terrain generation. Initial descriptive worldgen cannot
advertise this complete feature on its own.

## 9. Faction-authored places and staff approval

A faction leader uses §7's in-game surveys to propose a base, Elysium, corporate headquarters, or
another place expressing the faction's identity. Use the same brief and manifest machinery, with the
[template's faction proposal section](neighborhoods/TEMPLATE.md#faction-place-proposal). Most such
places are **sites inside an existing neighborhood**; a campus or territorial home district can
justify a whole neighborhood. Neither room count nor faction prestige requires promoting a
building into a neighborhood.

The owner specifies **LLM review followed by a staff decision** on resourcing, style, and power
level. Unlike ordinary site hydration, a faction facility waits for that decision before becoming
active. Generated drafts can be inspected in staging. The submitter's faction authority, staff's
content approval, and authorization to spend faction resources are separately checked.

### What the proposal must make explicit

| Area | Required information |
|---|---|
| Representation and authority | Faction/organization IDs, submitting character, relevant role/delegation and scope, proposed owner, operator, and access administrator. A leadership title alone does not authorize spending or land acquisition. |
| Place and fiction | Neighborhood/site scope, footprint and entrances, architectural style, history, public role, private areas, and how players can engage with it. Identify an existing off-screen holding versus a proposed purchase, lease, renovation, or new construction. |
| Resources | Existing rights/assets, proposed source accounts or entitlements, acquisition/build costs, materials, labor, capacity, transport, time, and recurring rent/payroll/maintenance. Label each quantity as authoritative, quoted, estimated, or unknown. |
| Capabilities and power | Room and terrain budget; NPC roles, number, sheets and authority; guards, security, wards, sensors, storage, production, services, transport access, and other functional advantages. State limits, supported definitions, and recurring inputs. |
| Constraints and consequences | Dependencies, meaningful access restrictions and weaknesses, effects on neighboring play, and what happens when funding, staffing, rights, or services lapse. Approval of a building does not automatically grant control of its neighborhood. |
| Visibility | Public-facing descriptions, faction-private details, staff-only material, and which version of the proposal/report each audience may see. |

Review the proposal as a whole: separately modest guards, surveillance, walls, escape routes, and
income can combine into an excessive advantage. Security terrain and NPC behavior are part of the
capability review, not just descriptive color. Fictional claims such as "impregnable" do not create
rules, and an Elysium designation does not itself confer mechanical immunity. Explicitly list which
effects are social conventions, staff-run arrangements, or implemented mechanics.

An existing holding should cite its established entitlement and only resource the new additions;
it should not be charged for acquisition twice. Equally, calling a new fortress "always here" does
not establish prior title, paid construction, or trained staff. World records and staff's explicit
decision settle that distinction. Existing faction assets remain available according to their
current rights while an upgrade proposal is under review.

### Review and activation workflow

1. **Develop and approve the proposal.** The leader answers proposal-based surveys, evaluates
   feedback, and explicitly approves a concrete revision through §7. This author approval compiles
   the brief and resource/capability schedule for staff; it neither spends faction funds nor
   reserves unlimited land or industrial capacity.
2. **Prepare the staff review packet.** Validate references, authority, proposed geometry, budgets,
   and supported definitions. Include the actual brief, proposed topology/placement preview,
   intended NPC profiles and capabilities, resource schedule, and explicit unresolved questions.
   Attach advisory LLM findings on style, duplication, contradictions, affordability/upkeep,
   combined power, and opportunities for play, with evidence/policy references and suggested
   revisions. Reuse survey findings only where they remain valid for this brief and its
   dependencies. Estimates remain labeled; the model does not establish balances, rights, or
   prices. Full room and tactical-asset generation can wait for staff's decision on the brief.
3. **Staff decision.** Approve, request targeted changes through another author survey, or decline,
   recording reasons and conditions. Approval binds the exact brief revision, resource plan,
   capability limits, and relevant dependency revisions. An unfunded condition remains pending;
   stylistic approval is not funding authority. Store the decision separately from the author's
   approval and the LLM's recommendation.
4. **Generate to the approved specification.** Produce the full staged manifest, rooms, tactical
   terrain, NPCs, and exact world/resource diff within approved constraints. Validate the complete
   candidate before activation. A material divergence returns to the proposal/review path; a
   faithful implementation needs no second ritual of accepting every room. Staff can inspect
   previews, and any expressly required final review remains governed by publish policy.
5. **Execute and activate.** Revalidate current authority, approvals, balances, rights, and
   dependencies before reserving/consuming resources through normal org/economy operations.
   Acquisition, delivery, and construction follow their supported clocks, contracts, and work
   orders. Activate only the approved, completed stage and its funded capabilities; prose cannot
   skip the construction interval. Instantiating a verified existing off-screen holding can follow
   its approved hydration path without inventing a new construction event. Commit activation and
   associated economic effects idempotently through the authoritative services.

Content approval is not a replacement for an account's spending approval or current scoped
authority. Resource changes while review is pending must be checked at execution; the same funds
cannot finance two proposals. Recheck authority when a leader loses their role or staff approval
is revoked before execution. Any exceptional staff grant or policy exception must be explicit and
audited through its normal authority, with its extent visible in the decision record.

Keep approval, execution progress, and live-site status separate. Failure or delay leaves a durable
proposal with a clear reason; an incomplete facility is not advertised as operational. A staged
project can open its completed public hall while a later annex remains unavailable if staff approved
that sequence. Later loss of upkeep follows the approved consequence policy, such as reduced guard
coverage or suspended services, while preserving the place and its scene history.

The reviewer receives only authorized context. Faction-private plans and staff-only world conflicts
must not leak through a player-facing report. Use separately filtered reports and actionable
redacted feedback; do not submit a rival faction's secret defenses to a broadly visible review.
Neither the reviewer nor the authoring model can grant capabilities, approve itself, or execute
arbitrary commands from the brief.

### Example: an Elysium in a converted arts hall

The proposal specifies a leased site inside an existing neighborhood, a public gallery, a meeting
hall, private offices, and a faction host NPC. It names the lease entitlement, fit-out funding,
staffing and recurring costs, visitor policy, and the intended Elysium social rules. Any guards,
supernatural defenses, restricted escape route, or protected storage are separate explicit
capabilities. The reviewer can suggest keeping the gallery open for broader play and flag an
unfunded security plan; staff decide the approved scope. No setting-specific price, NPC strength,
immunity, or ownership right is inferred merely from the name "Elysium".

Approved sites use the same durable registry and Git export/reimport path as hydrated additions.
The repository stores the authored baseline and approved manifests; current money, title, memories,
damage, and operating state remain authoritative live records. This entire faction proposal flow
is a design extension; the existing organization/economy seams do not imply a shipped construction
or LLM review interface.

## 10. Existing seams and implementation order

| Existing implementation | Reuse | Required extension |
|---|---|---|
| `mush-core` ordinary rooms, exits, locks, zones | Local play and standard builder/movement validation. | Typed geographic classification and connections, without nesting rooms in rooms. |
| `mush-worldgen` theme resolution and reviewed descriptions | Neighborhood style, site/room overrides, metering, draft generation. | Brief/manifest planner, identity mapping, topology validation, staging, diff/apply; bounded player hydration and subsequent builder review. |
| `mush-jobs` durable queues and typed approval payload handlers | Staff assignments, notifications, comments, and decision dispatch. | Revision-bound proposal/brief payloads and authorized build decisions; persistent survey state belongs to the authoring feature, not free-text ticket comments. |
| `@gen/building` BFS within a zone (bounded hops if unzoned) | Existing manual building workflow. | Explicit site room sets for the new builder so a neighborhood-wide zone cannot pull in neighboring buildings. |
| `mush-sim` placement, interiors, domains | Authoritative physical placement and transitions. | Geographic transforms/coverage and, when needed, generic presentation resolution. |
| `mush-sim::route` room-list `Journey` for plain objects | Existing watchable movement and recovery behavior. | Typed geographic legs, player/vehicle travel integration, checkpoints, encounter promotion. Current `@route` cannot traverse roomless geography. |
| `mush-npc` profiles and tiered lore | Individual persona, shared local lore, bounded runtime actions. | Build-time profile/lore compilation with explicit scope and secret filtering. |
| `mush-faction::Region` | Territory/control overlays, including non-geographic influence. | Explicit links to geographic areas. Do not reinterpret its `RegionId` or `zones` as atlas coordinates. |
| `mush-org` scoped authority and `mush-economy` plans/approvals | Faction representation, spending authority, rights, resource checks, and durable economic changes. | Revision-bound faction place proposals, advisory LLM reports, staff content decisions, and integration with supported construction/service workflows. |

The atlas needs its own geographic identity type/namespace. Existing faction regions may cover
zones or abstract influence and must remain usable without any geographic footprint. Geographic
membership also cannot be reconstructed solely from zone membership: building zones and permission
boundaries may differ from neighborhoods. Reuse theme resolution deliberately, without silently
changing zone delegation when a site or neighborhood is reassigned.

Proposed build order:

1. Geographic records, stable IDs, membership queries, atlas projection, and manual connections.
2. Local and inter-neighborhood travel through implicit geography, with lawful observation and
   interception checkpoints. Prove two authored neighborhoods separated by one implicit area.
3. Open-to-play presence with server-side filtering and inference-oriented tests.
4. In-game proposals/surveys, explicit feedback evaluation, author-approved brief compilation,
   and typed staff submissions; versioned brief/manifest validation, staging, idempotent apply,
   and room/NPC generation. Proposal authoring and staff consideration can ship before the full
   tactical builder.
5. Tactical terrain linkage and transitions using the established geographic records. Keep the
   transform/entrance contracts in earlier briefs so this does not require rebuilding geography.
6. Player-requested site hydration using the builder and tactical terrain support together:
   persistent requests, quota/footprint reservations, complete activation with pending review,
   human correction, and export/reimport without loss or duplication.
7. Faction place proposals with resource/capability schedules, generated review packets, advisory
   LLM review, staff decisions, and authorized execution. This shares the builder foundation and
   can proceed alongside step 6 as the relevant economy/construction support becomes available.

Acceptance scenarios include: an adjacent but uncrossable river; distinct foot/road routes; a
timed rural path with no filler rooms; a private interior absent from public activity changes;
vehicle passengers retaining one location; a mid-route closure/restart; a checkpoint interception;
a secret entrance omitted from maps/prompts; a building batch staying inside its site; an unchanged
manifest applied twice; a staff edit/damage surviving regeneration; an implicit neighborhood built
without moving its neighbors; and an encounter spanning two neighborhood footprints.

Hydration acceptance scenarios: reuse an existing discoverable park without a charge; concurrent
requests/retries for one implied site yield one activation/charge; distinct valid parks remain
possible; hidden/reserved sites stay undisclosed; partial terrain generation leaves no reachable
half-site; a complete pending-review park is usable and survives restart/inactivity; quota renewal
does not delete sites; a long review backlog bounds new work without expiring existing places;
an occupied duplicate retains history through reviewed consolidation; and a base-brief rebuild
followed by hydration export/reimport preserves the site's IDs and live state.

Faction proposal acceptance scenarios: an unauthorized faction spender; an unfunded but stylish
headquarters; several modest features with excessive combined power; a private conflict omitted
from player-visible feedback; a staff-approved plan changed before execution; two proposals
competing for the same funds; a lost leadership role; an existing holding avoiding duplicate
acquisition charges; unfinished construction remaining unavailable; a public-hydration request
seeking faction-grade capabilities routed to staff review; and a funded approved site built once
with its limits, rights, recurring obligations, and subsequent play preserved.

Survey acceptance scenarios: restart resumes the same decisions; saved answers trigger no LLM
call until evaluation; follow-ups depend on prior choices; an unanswered default stays unaccepted;
a changed answer visibly reopens dependent decisions; stale async results cannot overwrite edits;
author approval compiles and submits once without granting staff approval; unauthorized synthesis
changes return to the author; staff-requested edits produce a new brief and diff; terminal jobs
are linked to new revision tickets instead of reopened; and no compilation/provider failure sends
a partial brief or silently charges faction construction resources.

Remaining decisions are terminology acceptance, WoD travel pacing, presence consent/bucketing,
hydration quotas and neighborhood expansion defaults, and final command names/survey presentation
and import/publish interfaces.
Faction review policy must also define resource evidence, power benchmarks, staff authority, and
the first supported acquisition/construction/service bindings.
These can be settled independently of the owner's central
requirement: focus playable detail into neighborhoods while preserving the geography between them.

## 11. Owner rulings (2026-09-09, binding)

Recorded from the owner's response to the [review](11a-neighborhood-geography-review.md). They
override the text above where the two conflict; the review's decision table (11a §8) is updated to
match. R1–R3 are decided. R4 is the owner's direction with the review's worked shape; the shape is
open to edits, the direction is not.

**R1. Worldgen digs; trust but verify.** Supersedes doc 03 §10 item 5 and the "never invents
topology" rule of doc 03 §5.2–5.3 (README "Superseded decisions" 3). The building agent may
propose everything a trusted builder may create: rooms, exits, sites, structures, features, NPC
profiles and lore, hooks, and their descriptions. The intended flow is the one this document
sketches in §7 with the file as an input: an author describes a neighborhood in a file (R2) or
in-game; the model validates it against the template's rules and the neighborhood's constraints,
asks clarifying questions, and then builds it. What stays, because it is verification rather than
distrust: the deterministic validator between proposal and world (geometry, topology, budgets,
policy, secrets, supported definitions); the typed-proposal boundary of doc 03 §7 (the model emits
typed records, never command text); metering, audit and kill switches; and human review *after*
activation where the neighborhood's policy calls for it. Standing principle: a restriction whose
only purpose is to keep an LLM from doing what a trusted builder may do is not load-bearing; when
one blocks this model it is struck in favor of trust-but-verify, and the strike is recorded here.
Site-kind templates (review H5) are authoring aids, never a constraint. Review D-11a-8 is decided.

**R2. Briefs and manifests are JSON.** §7's "Markdown with small YAML front matter" was a slip; the
agreed format is JSON, in the pack shape the tree already digests (`ruleset-tacsim` `PackManifest`
plus entries; review §3, M1, M3). The [template](neighborhoods/TEMPLATE.md) and the two examples
remain the *field reference* until the neighborhood pack schema lands (review §7 cut 0/1), and are
then re-expressed as JSON packs; the starter area (R4) is the first real pack. Markdown may exist
as a rendering for reading, never as the digested artifact. Review D-11a-3 is decided.

**R3. Quotas are character-scoped and count outstanding reviews, not creations.** Accounts are not
principals (doc 10 C1A, R3, R7), so there is no account-level counter; the review's H6 account-lane
proposal is withdrawn. A requester does not own what they hydrate, so a successful site is never
charged against them for good. The limit is a per-character cap on hydrations that are activated
and still `pending_review`, plus any still generating; a reviewer's approval or correction releases
the slot. Compute stays on the requester's actor budget (`BudgetScope::Actor`) under the builder's
own feature route. Neighborhood and locality caps bound the map (§8; review §6 "Sprawl"). Review
D-11a-7 is decided, differently from its recommendation.

**R4. The starter area is the reference fixture.** The review's first finding was that this
document specifies almost no data. The owner's answer is to build real data into the default
database: a starter area every new game boots with, which doubles as the standard test world for
every cut and as the demonstration of what an IC area looks like. Direction (owner): Limbo stays
the spawn room and gains a `Staff <S>` exit into a **Staff Lobby** neighborhood — a Hollywood-lot
tour, a behind-the-scenes area that shows what an IC neighborhood on a real game might look like —
and an **OOC Area** neighborhood, out of character by definition, where a RustMUSH tutorial lives.
Both are neighborhoods, so transit between them is a standard test; items and NPCs are dropped
throughout (a receptionist in the lobby, an off-shift gamemaster in the coffee shop).

What exists to build on: first boot seeds a minimal world of Limbo `#0` and God `#1` and writes
it through before the server accepts anyone (`crates/mush-server/src/persist.rs`); `[onboarding]
start_room` defaults to Limbo with an optional `start_room_unapproved` for the chargen gate
(`crates/mush-config/src/config.rs`); the tacsim seed pack shows a pack becoming objects through a
latch (`crates/ruleset-tacsim/src/seed.rs`). The starter area is therefore **a JSON pack (R2)
applied by the same loader and latch the neighborhood builder uses**, first-boot after `#0`/`#1`,
idempotent, re-applicable by staff, disable-able in config, retirable only through the reviewed
retirement path. It must never be hand-coded objects: it is the first proof that R2's format and
the builder's apply path work.

The worked shape (review's proposal; sizes are budgets, not targets):

| Piece | Content | What it exercises |
|---|---|---|
| Region `limbo` | A small metric frame ("the lot", 400 m square, flat datum, one raised loading dock) | frames, elevation datum, background terrain |
| Limbo `#0` | The spawn room, hub of the lot: exits `OOC Area <O>` (open) and `Staff <S>` (open by default so the tour is a tour; operators may lock it to staff in config) | boundary connections from a room in neither neighborhood |
| Neighborhood `ooc_area` (≤ 6 rooms) | Welcome Hall (arrival, signposts), Tutorial: Moving, Tutorial: Talking, Tutorial: Building (a sandbox where `BUILDER`-flagged players may `@dig`), Lounge, Tram Stop | the tutorial; a public neighborhood with no secrets; a `HydrationPolicy` allowing one player-hydrated site (a games room) so the hydration cut has a fixture |
| Neighborhood `staff_lobby` (≤ 7 rooms) | Lobby (receptionist), Coffee Shop, Tour Corridor, Back Lot (outdoor circulation with the loading dock), Writers' Room (restricted), Props Department (restricted; scenery and container demos), Tram Stop | private ingress / free egress; a restricted-room presence exclusion; a bar-level map (Coffee Shop) and a street-level map (Back Lot) for the substrate cut; a mezzanine floor for `dz` |
| Connection `lot_tram` | A journey connection between the two Tram Stops through implicit lot space, one checkpoint (Security Gate), served by a tram car — a carrier with a one-room interior — so the standard transit test is a real carrier journey with passengers aboard | the one travel system; checkpoints; a vehicle interior; premise 6 at toy scale |
| NPCs | `receptionist` (minor tier, Lobby: public lore — directions, hours, how to file `+request`; guarded — the tour schedule is running late; fallback lines), `gamemaster` (minor, Coffee Shop, off shift: public lore — where the tutorial is, what a scene is; jokes about being off shift), optional `docent` (extra tier, Welcome Hall) | lore tiers and pools; NPCs that work with `[llm].enabled = false` through fallback lines, so the fixture needs no provider |
| Items | A tutorial pamphlet (readable), the shooting schedule clipboard (details), a lost-and-found box (container), the notice board (details), a coffee cup | details, containers, scenery that must not become rooms |
| Secrets | One staff-only fact (why the tour is late) stored `Secret`, never in a prompt | the secret filter, end to end |
| Optional | A `Studio Security` faction with a control record over the Back Lot | `RegionRef::Atlas`, `+territory` |

Rules for it: total room budget under fifteen; every room justified by a tutorial step or a demo;
no supernatural or ruleset canon; the Staff Lobby's IC texture is a film lot, so it demonstrates
an IC area without pretending to be one game's setting. The pack is the fixture for the review's
§10 acceptance scenarios and for cuts 1–3 and 5 of its build order, in that order: import (cut 1),
tram and timed walk (cut 2), the games-room hydration (cut 3), the two maps (cut 5).
