---
brief_version: 1
id: neighborhood.lantern-ward
name: Lantern Ward
status: example
representation: authored
region: region.redwater-basin
locality: locality.redwater
setting: urban
coordinate_frame: region.redwater-basin.surface_m
origin_m: [0, 0, 0]
rotation_degrees: 0
footprint_local_m: [[0, 0], [600, 0], [600, 600], [0, 600]]
room_budget: {target: 9, maximum: 12}
external_dependencies:
  - region.redwater-basin
  - locality.redwater
  - neighborhood.eastbank
  - corridor.east-road
  - neighborhood.mill-creek/room.west-arrival
---

# Lantern Ward

Fictional urban example for the [template](../TEMPLATE.md). No campaign canon or runtime objects
are created by this file. External region/locality/corridor records are proposed companion content
and must exist before activation; the other authored endpoint is in [Mill Creek](mill-creek.md).

## Intent and scope

A former tram-maintenance quarter where late-shift workers, precarious tenants, and new nightlife
businesses depend on one another while disagreeing about who the neighborhood is becoming for.
Players come for conversation, repairs, introductions, and tenancy disputes. The courtyard and
bar are natural places for strangers to meet. Build the nine rooms below; remaining warehouses,
apartments, and shopfronts are background. Additional rooms require an amended brief or the bounded
hydration policy below. No adopted
ruleset or supernatural canon is assumed; supernatural explanations require campaign direction.

## Public look and feel

Warm bulbs hang below old tram-wire brackets; rain collects in grooves beside patched brickwork.
Shops have hand-lettered opening hours that change more often than the fading industrial signage.
By day, shutters clatter and repair work spills onto thresholds. At night, conversation carries
from the courtyard while distant freight shunts mark pauses in the music. Favor reused materials,
improvised hospitality, and signs of people maintaining things they cannot afford to replace.

Descriptions: present tense, objective third-person environmental prose, roughly 90–140 words per
room. Give each room two inspectable details. Avoid declaring the reader's feelings, universal
hostility, or generic decay in every space. The bar is meticulously clean; the workshop is ordered
despite its crowded shelves. Background patrons are ambience, not individually simulated NPCs.

## Geography, neighbors, and routes

The footprint is 600 m square in a fictional metric frame (+x east, +y north, +z up). The room
graph follows one east–west street and a courtyard spur. North/south/west expansion is background
and has no exit in this build. East Road crosses the implicit Eastbank neighborhood before
reaching Mill Creek. Eastbank occupies regional x=600–6200, y=0–1800 in this fixture; it has no
permanent rooms. The wider region may contain unnamed terrain outside these footprints.

| Connection ID | Local endpoint / anchor | Other endpoint | Kind / modes / direction | Route and policy |
|---|---|---|---|---|
| connection.east-road | room.east-arrival at [600,200,0] | neighborhood.mill-creek/room.west-arrival | journey; walk/drive; both | corridor.east-road; regional polyline [600,200,0] → [6200,200,0], 5600 m; public road through Eastbank, mode-based timed travel; no filler rooms |

The corridor is one shared connection record; Mill Creek references its reverse rather than
creating a second road. Its illustrative straight geometry is fictional. A crossing at regional
[3400,200,0] is a potential observation/interception checkpoint; nobody automatically learns who
crosses it. Campaign movement policy supplies speed/compression before a travel build is valid.

## Sites and structures

| Site ID | Name / kind | Local footprint | Entrance | Purpose / access |
|---|---|---|---|---|
| site.last-tram | Last Tram / bar | x=240–300, y=380–430 | room.bar via room.courtyard | Public meeting point; back room requires staff-issued access |
| site.relay-repair | Relay Repair / workshop | x=400–460, y=180–230 | room.workshop | Repair consultations; private office |
| site.courtyard-flats | Courtyard Flats / housing | x=280–340, y=420–480 | room.stairs | Public stair landing; private sample flat |

Each site has one building; no separate structure record is necessary in this draft. The bar has
amber glass, polished salvaged counters, and a chalkboard of favors owed in jest. The workshop
smells of warm insulation and citrus cleaner. The flats have individually repaired doorbells and
a shared landing planted with herbs. Ownership is descriptive background only, with no asset grants.

## Room intents and local topology

| Room ID | Name | Site / circulation | Anchor [x,y,z] | Access | Scene purpose |
|---|---|---|---|---|---|
| room.east-arrival | East Road Stop | circulation | [600,200,0] | public | A specific bus shelter and departure point, not all of Eastbank |
| room.junction | Tram Street Junction | circulation | [300,200,0] | public | Recognizable wayfinding and street encounters |
| room.courtyard | Lantern Court | circulation | [300,400,0] | public | Main gathering space, benches and shared noticeboard |
| room.bar | Last Tram Barroom | site.last-tram | [280,400,0] | public | Conversation and introductions |
| room.back-room | Last Tram Back Room | site.last-tram | [260,400,0] | restricted | Private planning; never included by default in open-play discovery |
| room.workshop | Relay Workshop | site.relay-repair | [420,200,0] | public | Negotiated repairs and practical favors |
| room.office | Relay Office | site.relay-repair | [440,200,0] | restricted | Records and confidential consultation |
| room.stairs | Shared Landing | site.courtyard-flats | [300,420,0] | public | Neighbor encounters, threshold conversations |
| room.flat | Sample Flat | site.courtyard-flats | [300,440,0] | restricted | A possible private scene location; no player tenancy assigned |

| Exit ID | From | To | Forward / return aliases | Access / distance |
|---|---|---|---|---|
| exit.east-street | room.junction | room.east-arrival | east;e / west;w | public street, 300 m; local travel policy |
| exit.court-spur | room.junction | room.courtyard | north;n / south;s | public lane, 200 m; local travel policy |
| exit.bar-door | room.courtyard | room.bar | bar / out | public westward doorway |
| exit.bar-back | room.bar | room.back-room | back / bar | visible door, ingress requires access; free egress |
| exit.workshop-door | room.junction | room.workshop | workshop / street | public frontage path, 120 m |
| exit.office-door | room.workshop | room.office | office / workshop | visible door, ingress requires access; free egress |
| exit.landing | room.courtyard | room.stairs | landing / courtyard | public entrance, ground-floor landing |
| exit.flat-door | room.stairs | room.flat | flat / landing | visible door, ingress requires access; free egress |

The public graph is connected. Private doors are known, their interiors and occupants are not
publicly inspectable. Default restricted access is staff-controlled until a real tenancy/service
policy is installed. Do not add unlisted upstairs rooms or intervening street filler.

## NPCs

- **npc.mara — Mara Venn**, Last Tram proprietor, site.last-tram, minor tier; starts in room.bar.
  Dry, precise, remembers practical preferences, dislikes public gratitude. Wants the bar to remain
  useful to locals but needs profitable evening trade. Fears becoming somebody's patronage project.
  Speaks in short questions; is privately enthusiastic about amateur radio. Knows public local
  directions and trading hours; guarded knowledge: the business is behind on maintenance payments.
  Does not know supernatural identities or tenant secrets. Respects Ivo's reliability and disputes
  June's approach to fundraising. Fallback: "Sit down. There's time to explain it properly."
- **npc.ivo — Ivo Sen**, technician, site.relay-repair, minor tier; starts in room.workshop.
  Patient with machines, impatient with vague requests, delighted by clever repairs. Wants steady
  work without becoming the local emergency service for free. Speaks concretely and asks what
  failed first. Knows workshop procedures; guarded knowledge: a repair log is incomplete. Does not
  know why it is incomplete. Relies on Mara for introductions and owes June a repair. Fallback:
  "Tell me what it did before it stopped working."
- **npc.june — June Calder**, tenants' organizer, neighborhood scope, minor tier; starts in
  room.courtyard. Warm, energetic, bad at accepting help; wants secure housing and fears dependence
  on a single benefactor. Remembers names but loses paperwork. Knows public routes and notices;
  guarded knowledge: two tenants disagree about pursuing a complaint. Does not know their private
  histories. Challenges Mara's caution and values Ivo's practical advice. Fallback: "Have a seat;
  I'm trying to make these dates agree."

NPC movement within their site or public backbone is future routine intent, not an implemented
schedule. Each gets a shared **public** local-directions lore pool and only their own guarded facts.
Current familiarity gates govern guarded sharing. No sheets, paid repair service, inventory,
relationships earned through play, or special abilities are generated by this example.

## Hooks and local tensions

- **hook.noticeboard:** June's meeting notices conflict with Mara's event booking. Invitations in
  room.courtyard, conversation at the bar, and workshop delivery receipts provide three approaches
  to establishing the dates. Players can mediate, organize separately, or leave it alone. Success
  may change later invitations; no scripted agreement is predetermined.
- **hook.repair-log:** Ivo asks for help reconciling a missing equipment entry. Players can examine
  the allowed copy, ask Mara about a delivery, or seek permission to inspect the office paperwork.
  The stakes are local trust and responsibility, not an automatic currency payout.

Both hooks are staff-run scene seeds pending supported bindings. No quest engine or reward is
implied by the prose.

## Staff-only knowledge and hidden spaces

**secret.log-gap:** a former contractor removed an entry to conceal an unauthorized loan of
equipment. The contractor (background, no generated NPC) knows the truth; the three authored NPCs
do not. Store as staff-only material, excluded from public prose and runtime NPC prompts. Staff
may reveal a dated duplicate through an investigation; only then add the discovered fact to the
appropriate knowledge records. There are no hidden entrances in this example.

## Play discovery and tactical grounding

Advertise only consenting open-to-play characters, aggregated at neighborhood level; the courtyard
and bar are suggested public destinations. Restricted interiors contribute no default signal.
Never publish where June or a player moved using the activity feed.

Street and courtyard are paved; the site rectangles above are provisional terrain footprints.
Exact doors, wall thickness, cover, and usable vehicle access require a later tactical terrain
pass. Do not treat room anchors as combat spawn points. The bar counter, workshop shutters, doors,
and noticeboard should receive stable feature IDs if modeled destructibly, and keep live changes.

## Builder acceptance and change ownership

Create exactly nine initial rooms, three sites, three NPCs, eight reciprocal local connections,
and the one shared external journey connection after dependencies resolve. Verify public walking
routes, private ingress/free egress, geography, separate site batch boundaries, distinct voices,
and secret filtering. Missing campaign travel policy blocks timed travel activation.

Source owns proposed topology, prose, and initial profiles/lore. Preserve staff edits through
conflict review and retain all live memories, scene results, inventory, property rights, and
damage. Reapplying the same manifest must be a no-op; no extra rooms or financial assets.

## Player-requested site hydration

Proposed allowance: one new pocket park, up to three additional rooms (twelve total across the
neighborhood), at most one pending site review, and at most one minor caretaker NPC. The quota
period and terrain/compute limits come from game-level policies and must be supplied before this
feature is enabled. The initial nine-room build does not automatically generate this park.

The available space is a publicly accessible background garden at local x=340–400/y=300–400,
east of Lantern Court, with a proposed access point at [340,380,0]. A path from room.courtyard
can reach it without crossing a building footprint. Keep neighboring flats, the workshop, existing
circulation, and ownership unchanged. The park should offer a sheltered conversation space with
evidence of local maintenance; no extra shops, private bases, or rewarding services.

Check for an existing or reserved park serving this garden before reserving it. The complete package
must include grounded walkable terrain, boundaries, cover, entrances, room mappings, descriptions,
and neighborhood discovery/presence integration. Add a caretaker only if useful, with public local
knowledge and an individual personality; do not grant access to staff-only secrets or invent past
player encounters. Terrain resolution must use a supported driver and the game policy's limits.

Activate the complete validated site as pending_review under the proposed delegated policy;
builders review fit and duplication afterward. Persist it without age/inactivity expiry. Register
it as a neighborhood addition immediately, then export its brief/manifest/assets for Git review.
Subsequent builds preserve it and all live consequences. Correct duplicates using the design's
reviewed migration policy, preserving occupied scenes and references. Review belongs to the
neighborhood's builder team, with a staff fallback if none is assigned.
