---
brief_version: 1
id: neighborhood.mill-creek
name: Mill Creek
status: example
representation: authored
region: region.redwater-basin
locality: locality.mill-valley
setting: rural
coordinate_frame: region.redwater-basin.surface_m
origin_m: [6200, 0, 0]
rotation_degrees: 0
footprint_local_m: [[0, 0], [1800, 0], [1800, 1800], [0, 1800]]
room_budget: {target: 8, maximum: 10}
external_dependencies:
  - region.redwater-basin
  - locality.mill-valley
  - neighborhood.eastbank
  - corridor.east-road
  - neighborhood.lantern-ward/connection.east-road
  - neighborhood.lantern-ward/room.east-arrival
---

# Mill Creek

> **Format superseded (doc 11 §11 R2, 2026-09-09):** briefs and manifests are JSON packs. This
> Markdown/YAML form stays as the *field reference* until the neighborhood pack schema lands, and is
> then re-expressed as a pack; it is never the digested artifact.

Fictional rural companion to [Lantern Ward](lantern-ward.md), using the same [template](../TEMPLATE.md).
External geography and movement policy are proposed fixture dependencies, not shipped content.

## Intent and scope

A farming community gathered around a store, a bridge, and a shared water problem. New residents
and established growers disagree over maintenance priorities while depending on the same crossing
and deliveries. Players come for supplies, introductions, practical help, and local disputes.
The common is a natural meeting point. Eight rooms cover the playable core within 1.8 km square;
fields, dispersed homes, and most riverbank remain geographically present background.

This is a rural neighborhood within a valley locality, with no requirement to call the valley a
city. No ruleset-specific supernatural explanation or simulated agriculture is assumed.

## Public look and feel

Low fences divide hayfields from vegetable plots. The old flood gauge is repainted more carefully
than the bridge railing; store deliveries leave clean rectangles on its dusty porch. Wind makes
conversation near the bridge intermittent, while the common is sheltered by mature trees. After
rain, tire tracks shine in the farm lane. At night, darkness separates the few occupied buildings
instead of making the countryside uniformly silent.

Use present-tense environmental prose, 90–140 words and two inspectable details per room. Favor
specific maintenance, distances, seasonal work, and neighborly obligations. Avoid depicting rural
NPCs as uniformly suspicious, quaint, poor, or technologically inept. Crop/weather details must
fit the campaign season; no automatic weather state is authored by this example.

## Geography, neighbors, and routes

The local origin is regional [6200,0,0]; axes and meters match Lantern Ward. Eastbank is implicit
between the two authored neighborhoods. Mill Creek's western arrival at local [0,200,0] is thus
regional [6200,200,0]. The stream follows approximately local x=160 north–south; the road crosses
at the bridge. Precise banks and elevation are deferred to terrain authoring.

| Connection reference | Local endpoint / anchor | Other endpoint | Kind / modes / direction | Route and policy |
|---|---|---|---|---|
| neighborhood.lantern-ward/connection.east-road | room.west-arrival at [0,200,0] | neighborhood.lantern-ward/room.east-arrival | journey; walk/drive; both | Reverse of the shared corridor.east-road, 5600 m; same checkpoint and campaign clock policy; do not duplicate the connection |

Fields do not automatically create public shortcuts. A flooded bridge closes the crossing and
requires a hold or a separately authored lawful detour, not movement straight through the stream.
Background fords are not usable routes in this initial build.

## Sites and structures

| Site ID | Name / kind | Local footprint | Entrance | Purpose / access |
|---|---|---|---|---|
| site.store | Mill Creek Store / shop | x=380–440, y=220–280 | room.store via room.common | Public gathering and supply inquiries |
| site.ash-farm | Ash Farm / farmstead | x=800–1100, y=180–500 | room.farm-gate | Gate and yard open for visitors; working barn and kitchen by invitation |

Ash Farm has **structure.farmhouse**, x=850–920/y=320–380, and **structure.barn**,
x=950–1020/y=320–400. Doors connect each to room.yard. The farm's fields extend beyond the site
footprint as background land use; no land ownership record or purchasable acreage is minted.
The store uses its site as its single-building identity. Its porch smells of feed bags and coffee;
the farmhouse has mismatched hooks for work coats; the barn has carefully labeled spare parts.

## Room intents and local topology

| Room ID | Name | Site / structure or circulation | Anchor [x,y,z] | Access | Scene purpose |
|---|---|---|---|---|---|
| room.west-arrival | West Road Lay-by | circulation | [0,200,0] | public | A particular pull-off for regional arrivals |
| room.bridge | Mill Creek Bridge | circulation | [160,200,0] | public, conditional crossing | Flood gauge, observation, disputed maintenance |
| room.common | Mill Creek Common | circulation | [400,200,0] | public | Main gathering place and local wayfinding |
| room.store | Store and Porch | site.store | [400,240,0] | public | One coherent small shop/porch scene space |
| room.farm-gate | Ash Farm Gate | site.ash-farm | [900,200,0] | public approach | Arrival, permission, waiting for an escort |
| room.yard | Ash Farm Yard | site.ash-farm | [900,340,0] | visitors welcome | Shared work and introductions |
| room.barn | Working Barn | site.ash-farm / structure.barn | [980,340,0] | restricted | Equipment consultation and private work |
| room.kitchen | Farmhouse Kitchen | site.ash-farm / structure.farmhouse | [880,340,0] | restricted | Invitation-only conversation |

| Exit ID | From | To | Forward / return aliases | Access / distance |
|---|---|---|---|---|
| exit.west-approach | room.west-arrival | room.bridge | east;e / west;w | public road, 160 m |
| exit.bridge-crossing | room.bridge | room.common | east;e / west;w | public road/crossing, 240 m; bridge closure gates passage |
| exit.store-door | room.common | room.store | store / common | public path, 40 m |
| exit.farm-road | room.common | room.farm-gate | farm / common | public approach, 500 m; timed local path, no field filler rooms |
| exit.farm-lane | room.farm-gate | room.yard | yard / gate | visitor lane, 140 m |
| exit.barn-path | room.yard | room.barn | barn / yard | 80 m path and door; restricted ingress, free egress |
| exit.kitchen-door | room.yard | room.kitchen | kitchen / yard | 20 m path and door; invitation ingress, free egress |

All meaningful distances use campaign movement policy, with explicit nonzero travel for the farm
road. Pausing on that route creates a particular roadside encounter if needed, not a persistent
"all the fields" room. No isolated farm rooms, extra bedrooms, or cross-country exits.

## NPCs

- **npc.tess — Tess Ash**, grower, site.ash-farm, minor tier; starts in room.yard. Direct, curious
  about unfamiliar tools, guarded about accepting favors. Wants a reliable crossing before the
  next delivery and fears debts of obligation more than hard work. Her careful plans coexist with
  impulsive generosity. Knows farm access and public road conditions; guarded fact: she missed a
  maintenance meeting. Does not know who changed the meeting notice. Fallback: "You can help me
  hold this, or we can sit down and talk."
- **npc.ren — Ren Vale**, storekeeper, site.store, minor tier; starts in room.store. Sociable,
  punctual, and unexpectedly sentimental about old packaging. Wants deliveries to remain viable;
  fears being treated as the community's automatic creditor. Uses exact dates rather than broad
  accusations. Knows public delivery schedules; guarded fact: one delivery was privately deferred.
  Does not know customers' secret finances. Fallback: "I have the date written down somewhere."
- **npc.ada — Ada Morrow**, volunteer water observer, neighborhood scope, minor tier; starts in
  room.common. Patient in disagreement, fierce about recording measurements, fond of terrible
  jokes. Wants trustworthy flood records and fears a preventable accident. Knows public gauge
  readings; guarded fact: one reading was reconstructed after a missed visit. Does not know
  whether that reading caused the current discrepancy. Fallback: "We can check the marks together."

Tess trusts Ada's care but questions her delivery priorities; Ren wants Tess's trade and declines
to extend informal credit; Ada buys supplies from Ren and wants public access to the records.
Give each the shared public-directions pool and only their own guarded facts. Future routines can
cover their site and the public backbone, but no movement schedule is implemented by this brief.
No sheet, inventory, bank balance, automated credit, or mechanical farming service is requested.

## Hooks and local tensions

- **hook.high-water:** a discrepancy between delivery reports and the gauge creates pressure to
  inspect the bridge. Reading the visible marks, asking Ren about dates, and comparing Ada's
  permitted records provide independent approaches. Players can investigate, organize maintenance,
  seek another supplier, or decline. Staff resolve closures and any lasting repair effects.
- **hook.shared-tools:** Tess needs a tool Ren has promised to someone else. The yard conversation,
  store notices, and Ada's recollection of an earlier agreement offer ways to mediate. Negotiation
  need not end with a shared solution. No free equipment or economic transfer is generated.

These are staff-run hooks until a real scene/contract/clock binding is supplied.

## Staff-only knowledge and hidden spaces

**secret.old-mark:** an older gauge backing was installed slightly out of alignment. A retired
installer (background, no generated NPC) knew this; Tess, Ren, and Ada do not. Store as staff-only
source material, excluded from runtime NPC and public-prose prompts. An approved inspection can
reveal a mounting discrepancy and then add that observation to the participants' knowledge.
This is a possible explanation to investigate, not a predetermined finding from any player action.

The barn and kitchen are private but their doors are visible. There are no secret tunnels,
unlisted rooms, or publicly advertised interior occupants.

## Play discovery and tactical grounding

The common and store are public meeting suggestions. Only consenting open-to-play characters
contribute to a coarse neighborhood signal; private interiors and passing travelers do not.

Preserve stream/bridge alignment, road widths sufficient for ordinary deliveries, and the distinct
farm buildings. Exact banks, elevations, doors, fence openings, and cover require tactical terrain
authoring; current footprints are estimates. A bridge encounter can use a small terrain patch
without simulating all 3.24 km² of the neighborhood. Damage to the bridge, gate, structures, or
gauge must survive a description rebuild once those features acquire durable identities.

## Builder acceptance and change ownership

Create exactly eight rooms, two sites, two farm structures, three NPCs, and seven reciprocal local
connections. Reference the existing shared regional connection. Verify transformed arrival
coordinates, timed rural movement, bridge gating, private ingress/free egress, connected public
paths, distinct voices, and secret filtering. Missing external records or travel policy prevent
travel activation; no guessed destination or fabricated command is acceptable.

Source owns initial descriptions, topology, and profiles/lore. Live relationships, memories,
inventories, land/property rights, closures, repairs, and scene outcomes are preserved on rebuild.
Unchanged manifests apply without new objects or changes. The room budget is not a request to fill
the spare two slots.

## Player-requested site hydration

Proposed allowance: one riverside picnic park, up to two additional rooms (ten neighborhood rooms
total), at most one pending site review, and no dedicated NPC by default. The game must supply
creation quota, compute, and supported terrain-driver limits before enabling the feature. The
initial eight-room build leaves the park implicit until requested.

Available footprint: local x=220–320/y=260–380, on the common's side of the stream, with an entrance
at [320,260,0] connected by a public footpath from room.common. Keep the bridge as the only authored
stream crossing; the park must not produce a ford or bypass a closure. It also must not consume
Ash Farm's land. A small gathering place and a distinct waterside viewpoint are sufficient.

Check for an existing or pending picnic site before reserving the footprint. Require complete
descriptions, exits, access and room mappings, working tactical terrain, and neighborhood integration
before activation. The selected terrain must preserve the established stream/bridge geometry, give
the park physically consistent elevations, boundaries, and cover, and meet the game policy's size
and resolution limits. Existing NPCs may acquire appropriate public knowledge of the park through
the normal lore update path; grant no secrets, personal memories, equipment, or ownership.

Complete validated activation may precede human review with pending_review set. The site persists
through inactivity, restart, and quota renewal. Register the addition durably and export its source
and manifest for Git reconciliation; a base-brief rebuild cannot remove it. Neighborhood builders
(staff fallback) review fit or duplication and preserve ongoing scenes, references, and live state
through any correction or consolidation.
