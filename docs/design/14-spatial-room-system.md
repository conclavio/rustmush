# 14 — The integrated spatial/room system

**Draft 0.1 — 2026-09-09.** The design proposal for one spatial system in which rooms, exits,
terrain, elevation, travel, tactical maps and the content that authors them are records on a
single substrate. Written from first principles off the givens and the owner's answers to
[14a](14a-spatial-owner-questions.md) (all recommendations accepted with seven sharpenings, §0),
not from doc 11's text; where doc 11 and this document differ, this document governs (14a Q29).
Code cited at `0bc5e92` by path and symbol; no line numbers. Nothing here is built.

Companions: [14a](14a-spatial-owner-questions.md) (the survey and its answers),
[11](11-neighborhood-geography-and-authoring.md) (the neighborhood direction and rulings R1–R4),
[11a](11a-neighborhood-geography-review.md) (what the tree holds), 06 (placement graph,
transitions, domains), 07 §9 (mobility services), 10 (scales, journeys, character-first),
09 C8 (reservations), 09b (lanes), 03 (LLM layer, typed actions).

---

## 0. Decisions this document rests on

The givens G1–G9 of 14a §1 and these answers (owner, 2026-09-09, binding):

| # | Decision | Source |
|---|---|---|
| D-14-1 | Scope: substrate, packs, apply path, starter area, the agent's contract; not the survey UX, presence, or faction places | 14a 1A |
| D-14-2 | Street level (1–2 m) is the first tactical consumer; the TacSim ground driver follows on the same substrate | 2A |
| D-14-3 | Rooms stay the unit of social play; geometry is attached; the room a pose is in is derived | 3A |
| D-14-4 | One metric substrate per region; the canonical pose is metric; **square grids only — hexes are struck from every map, driver and document**, because a hex cannot be shown honestly in a text terminal | 4A, sharpened; I8 |
| D-14-5 | Metric surface regions; node graphs for orbital, system and interstellar space; carrier interiors have local frames | 5A |
| D-14-6 | Layered 2.5D elevation: ground heightfields plus structure floors as layers; no voxels | 6A |
| D-14-7 | Resolution is per tile; a window chooses one cell size; never mixed | 7A |
| D-14-8 | Deterministic procedural background for implicit space, stored only when something durable happens there | 8A |
| D-14-9 | Features carry damage; cells are immutable in v1 | 9A |
| D-14-10 | **The engine defines the world**: terrain classes, locomotion classes and their movement effects, line of sight, projectile and energy propagation are engine concepts; rulesets register definitions (units, weapons, damage) that consume them and may extend the vocabularies | 10A, sharpened |
| D-14-11 | Local movement is instantaneous by default; a portal's policy makes a specific exit timed | 11A |
| D-14-12 | One travel system grown from the carrier journey; `route.rs`'s room-list journey retired | 12A |
| D-14-13 | Rides (no vehicle object) and carriers (vehicles and ships with interiors) both exist | 13A |
| D-14-14 | Journeys run on `WorldIc`; real time is IC time over the campaign rate, with a per-checkpoint real dwell floor and a per-mode real cap; WoD campaigns run IC time at a multiple of real time | 14A; I2 |
| D-14-15 | Checkpoints are coordinates; a stake-out materializes one | 15A |
| D-14-16 | `+travel` over known destinations; the server chooses the route | 16A |
| D-14-17 | A tactical domain is a scene-driven window over pre-authored tiles; staff may open explicit windows | 17A |
| D-14-18 | Rooms remain the audience inside a domain; the resolver decides which | 18A |
| D-14-19 | Oversized windows coarsen rather than refuse | 19A |
| D-14-20 | One pack per neighborhood, one per region; hydrated sites are addition packs; references are `(pack, entry)` | 20A |
| D-14-21 | The agent accepts prose, a partial pack or a complete pack and always produces a complete pack | 21A |
| D-14-22 | A bounded off-loop agent with a validator tool; **$5 hard cap per build, $2 target** | 22A; I3 |
| D-14-23 | Staff builds go live on the staffer's word; player hydration goes live as `pending_review` | 23A |
| D-14-24 | Clarifying questions live on a durable proposal record, answered in-game or on the web | 24A |
| D-14-25 | Hydration cap of **2** outstanding reviews per character, **staff-settable per character**; trash earns strikes and strikes drive the cap to zero; good conduct earns a higher cap | 25A, sharpened |
| D-14-26 | The starter area ships **built** (no LLM at boot) **and with its brief**, so a staffer can reconceptualize it by running the authoring workflow from the same brief with different answers | 26A, sharpened |
| D-14-27 | `Staff <S>` is open by default; back rooms restricted; the lot is a film lot | 27A |
| D-14-28 | Theme and weather key on the atlas; zones keep locks and exits; control regions and properties gain atlas references | 28A |
| D-14-29 | Names: `mush-atlas` over `mush-content`; `+atlas`, `+travel`, `+stakeout`, `@atlas/*`; tiers region / locality / neighborhood / site / structure / room; "control region" for the faction record; docs 14, 14a, 14b, 14c | 29A |
| D-14-30 | **SSH first for maps**: anything the terminal can show the web can show, never the reverse; the map projection is a text grid and the web draws the same frames | I7 |
| D-14-31 | Boot never spends: the starter area is static committed content; the agent path is never a boot step; test instances do not apply it | G9 |
| D-14-32 | Interface NPCs: staff-zone NPCs as the conversational face of game systems, holding only the asker's authority and only read-or-propose abilities | owner, **decided 2026-09-09** (§4.6); 14b's first slice |

Kept from the binding designs, unchanged: doc 06's single authoritative placement graph and
atomic transitions; doc 10's abstracted routes (1A), journeys never pause for scenes (6A), carrier
roots move their topology (C16A), background journeys (C19A), interception promotes into a live
domain (C20A), one active focus (C21A); doc 09 C8; doc 09b lanes; doc 03 §7's typed-action
boundary. Opened and changed here: doc 06 §5.1's opaque pose (D-14-4), doc 06 §3.3's boundary
(D-14-10), and every hex reference in docs 04, 06 and 10 (D-14-4).

## 1. The model, from first principles

### 1.1 What must be true

1. **Play happens in rooms** (G1, D-14-3). `look`, `say`, exits, locks, contents, NPC wakes, scene
   anchors and property are room-keyed today and stay so. Whatever else exists, a character is
   always in exactly one room.
2. **Every room is somewhere** (G2, G4). A room has a place on a map: a footprint on one walkable
   surface, in a frame shared with every other room of its region. Two rooms' relative position is
   a fact, not a convention.
3. **The map does not stop at the rooms** (G1, G2, G6). Between neighborhoods, around landing
   sites and along corridors there is ground with height and cover that nobody has authored. It
   must be there the moment it is needed and cost nothing until then.
4. **Height is a property of the ground and of floors, not of rooms** (G4). Rooms sit on
   surfaces; surfaces have elevation; stairs, ramps and decks connect surfaces.
5. **A fight is a window** (G2, D-14-17). A tactical domain shows a bounded part of the map at one
   cell size; it does not own the map, and what it damages stays damaged in the map afterwards.
6. **Movement between places is one thing** (G1, G6, D-14-12): a character walking the farm road,
   a taxi across the city, a hauler across the Belt are the same record with different travelers
   and modes, on one clock, with the same rule that nothing skips a place where it could be seen.
7. **The engine owns physics; rulesets own definitions** (D-14-10). What a surface is, how a
   biped or a tracked vehicle crosses it, what blocks sight, how a bullet, a shell or a beam
   travels are engine facts. What a weapon does with a clear line is a ruleset's.
8. **Content is data the engine can check** (G7, G8, G9). A neighborhood is a JSON pack; the
   agent that writes one is verified by the same validator a human's pack is; the world at first
   boot is a pack; nothing at boot costs money.
9. **The terminal is the floor** (D-14-30). Any map the game can show, it can show in text.

### 1.2 The concepts

Ten concepts carry all of the above. Everything else is a projection.

- **Region.** A bounded frame with a kind. A *surface* region is metric: metres, `+x` east, `+y`
  north, `+z` up, an origin, an optional geographic reference. *Orbital*, *system* and
  *interstellar* regions are node graphs: their places are nodes, their distances are connection
  durations, and they have no cells. Above the surface the constellation is **fixed**: a
  connection's distance is derived from real geometry at the campaign epoch and pinned (doc 10
  T17); no body moves, and the solar system is a set of placed neighborhoods, never a coordinate
  grid. A surface region owns a ground field (1.3) and a background recipe (1.2 **Background**).
- **Surface.** A walkable 2.5D layer in a region: the **ground** (layer 0) or a **floor** of a
  structure (layers above and below ground, each with its own height map and holes). A bridge deck,
  a mezzanine, a ship's compartment deck and a station ring are floors of structures. Every pose
  is on exactly one surface.
- **Tile.** A rectangular window of **cells** on one surface at one resolution, immutable per
  revision. A cell holds a surface class, an elevation, a cover class and a derived room index.
  Ground tiles are owned by the region (coarse) or a neighborhood (fine); floor tiles by their
  structure. Passability is not stored: it is the engine's locomotion table applied to the class.
- **Background.** The deterministic recipe that yields a cell for any `(surface, x, y)` no tile
  covers: a seeded function of the region's parameters (surface mix, relief amplitude, water
  level) and the coordinates. It is never stored unless a feature or damage lands on it.
- **Feature.** Static geometry with durable state on a surface: walls, doors, windows, counters,
  trees, gates, bridges' railings, craters. Features block movement or sight or both, occupy a
  height range, and carry `Intact | Damaged | Destroyed`. Destruction lives here, never in cells.
- **Room geometry.** A room's footprint: a set of cells on one surface (authored as a rectangle,
  a polygon, or an anchor with a radius and rasterized). Footprints of different rooms on one
  surface do not overlap. Cells in no footprint are background or circulation the author did not
  name.
- **Portal.** The geometry of an exit: a segment on the boundary between two footprints (or a
  point), a mode (door, gate, stair, ramp, ladder, hatch, open), a `dz`, a distance and a time
  policy. The exit's lock stays the authority; the portal only says where and how. A stair is a
  portal whose two ends are on different surfaces.
- **Connection.** A region-owned edge between two **access points** (a room and a point on its
  footprint). *Boundary* connections join adjacent footprints across a neighborhood line; *journey*
  connections cross implicit space along a corridor with timed legs and **checkpoints** (points on
  the corridor). A connection whose far room is not yet built is inactive, not invalid.
- **Journey.** One record for every traveler: a character, a plain object, or a carrier root whose
  interiors and occupants ride with it. It names a connection and a mode, is stamped on `WorldIc`,
  advances leg by leg with an idempotent cursor, and arrives at the far access point's room.
- **Window** (a tactical domain). A region, a set of surfaces, a bounding box and a cell size;
  its composite is the substrate sampled at that size, with features and the room index; it pins
  the tile revisions it read. Poses are metric; cells are derived. Windows write back feature
  states and object poses, never tiles.

### 1.3 Invariants, as properties to test

- **P1 One room.** Every character is in exactly one room at all times, journeys and windows
  included; the room is a projection of pose or of journey progress, never a second truth.
- **P2 One surface.** Every pose is on exactly one surface, and `z` equals that surface's height
  at `(x, y)` plus the pose's declared height above it.
- **P3 Footprints partition.** On one surface, no cell belongs to two rooms; every authored room
  has at least one cell; every portal touches both of its rooms' footprints (or is a journey
  portal, in which case the distance is explicit).
- **P4 Background is pure.** `background(region, surface, x, y)` is a pure function of the
  region's recipe and its inputs; two calls never differ; nothing is stored for it until a
  feature or damage exists there.
- **P5 Composite is pure.** A window's composite is a pure function of the tile revisions it
  pinned, the feature states, and its own frame; regenerating descriptions, re-applying a pack, or
  closing a window changes no cell and no feature state.
- **P6 Never skip.** While the server runs, a journey visits every checkpoint of its connection in
  order at or after its due time; a catch-up after downtime advances to the correct position in
  one move with no narration; a stake-out at a checkpoint always sees the traveler pass.
- **P7 One clock.** Journey progress, window ticks and scene time compare only through
  `ClockStamp`s of the same generation; a rate change re-anchors journeys and never strands one.
- **P8 Apply is idempotent.** Applying an unchanged pack creates nothing and changes nothing; the
  latch maps every pack entry to its object; retiring an entry with live references is refused
  with the references named.
- **P9 Boot is free.** First boot makes no LLM call; the testkit's minimal world has no starter
  area unless the suite asks for it.
- **P10 Text is sufficient.** Every map frame the server emits renders as a text grid with a
  legend and loses no fact that a rules query would use.

## 2. Records

All records are documents in one `atlas` typed family under a registered owner and codec (doc
09b), keyed by pack-scoped `CanonicalId` (`[a-z0-9_.]`, 64 bytes); the seed/apply latch lives in
`mush-content`. Shapes below are the contract; field names are binding, Rust syntax is not.

### 2.1 Frames and geography

```rust
pub struct RegionRec {
    id: CanonicalId,            // pack-scoped, e.g. region.redwater_basin
    name: String,
    kind: RegionKind,           // Surface { body } | Orbital { body } | System | Interstellar
    frame: Option<Frame>,       // Some for Surface: units Metres, origin, axes ENU, geo_ref
    parent: Option<(CanonicalId, Transform)>,
    background: BackgroundRecipe,   // surface mix, relief amplitude, water level, seed
    ground: Vec<TileRef>,       // coarse ground tiles (10–25 m)
    revision: u64,
}
pub struct LocalityRec { id, region, name, kind: LocalityKind, footprint: Polygon2, revision }
pub struct NeighborhoodRec {
    id, region, locality, name, setting: Setting,           // urban | suburban | rural | station | …
    representation: Authored | Implicit,
    footprint: Polygon2,        // region metres, stored resolved (no transform at runtime)
    room_budget: Budget,        // { target, maximum }
    hydration: HydrationPolicy, // 4.5
    zone: Option<ObjId>,        // the ZMO it maintains for locks/exits, if any
    reviewers: Vec<PrincipalId>,
    tiles: Vec<TileRef>,        // fine ground tiles (1–5 m) over the footprint
    revision: u64,
}
pub struct SiteRec {
    id, neighborhoods: Vec<CanonicalId>, name, kind: SiteKind,   // registered taxonomy
    footprint: Polygon2, entrances: Vec<CanonicalId /* portal */>,
    property: Option<PropertyId>,
    origin: Authored | Hydrated { request: RequestId },
    review: ReviewState,        // n/a | pending_review | approved | needs_changes
    tactical_ready: bool,       // every room has a footprint on a tile, every portal has geometry
    revision: u64,
}
pub struct StructureRec { id, site, footprint: Polygon2, floors: Vec<Floor>, revision }
pub struct Floor { index: i16, z_offset_m: f32, height_m: f32, tile: TileRef }   // index 0 = ground level of the structure
```

### 2.2 Surfaces, tiles, features

```rust
pub enum SurfaceRef { Ground, Floor { structure: CanonicalId, index: i16 } }
pub struct TileRec {
    id: CanonicalId, region, surface: SurfaceRef, owner: TileOwner,   // Region | Neighborhood | Structure | Addition
    bbox: Rect2 /* region metres */, resolution_cm: u16,               // 50, 100, 200, 500, 1000, 2500
    cells: Cells,               // packed: class u8, elev_cm i16, cover u8, room u16 (derived)
    revision: u64,              // a new revision is a new document; old ones stay for pinned windows
}
pub struct FeatureRec {
    id, region, surface: SurfaceRef, kind: FeatureKind,    // wall | door | window | gate | counter | tree | rail | crater | …
    geometry: Geometry3,        // polyline | polygon | point, region metres
    z_range: (f32, f32),        // above the surface
    blocks: Blocks { movement: bool, sight: bool },
    state: Intact | Damaged(u8) | Destroyed,
    room: Option<ObjId>, revision: u64,
}
```

Bounds: a tile is at most 256×256 cells (one document, ≈ 320 KiB packed); a neighborhood at most
64 tiles; a structure floor one tile; features per tile capped; all under the K3 bundle limits.
Cells store no passability: `passable(class, locomotion)` and `cost(class, locomotion, slope)` are
engine tables (§5).

### 2.3 Rooms, portals, connections

```rust
pub struct RoomGeo {
    room: ObjId, id: CanonicalId, neighborhood, site: Option<CanonicalId>,
    surface: SurfaceRef, footprint: Footprint,   // Rect | Polygon | Cells(tile, set) | Anchor { point, radius_m }
    kind: Outdoor | Indoor | Circulation, revision: u64,
}
pub struct PortalRec {
    id, exit: Option<ObjId>, from: Endpoint, to: Endpoint,   // Endpoint = Room(id) | Outside(point)
    geometry: Segment3 | Point3, mode: Door | Gate | Stair | Ramp | Ladder | Hatch | Open,
    distance_m: u32, dz_m: i32, time: TimePolicy,           // Instant | Timed { mode_set } — 3.3
    visibility: Visibility, revision: u64,
}
pub struct AccessPoint { neighborhood: CanonicalId, room: Option<ObjId>, point: Point3 }
pub struct ConnectionRec {
    id, region, kind: Boundary | Journey,
    endpoints: (AccessPoint, AccessPoint), direction: Both | OneWay,
    modes: Vec<TravelMode>,     // registered: foot, car, transit, ship, shuttle, torch, …
    corridor: Polyline3,        // region metres; nodes' ids for node-graph regions
    legs: Vec<LegDef>,          // { to: Checkpoint, base_s: u32, hazard: u8, interdiction: u8 } — kept from route.rs
    cost: RulesValue,           // ruleset-owned cost policy (drive class, propellant, alignment band …)
    visibility: Visibility, status: Active | Inactive, revision: u64,
}
```

`LegDef`'s `hazard`/`interdiction` bands and `MAX_LEGS` = 32 carry over unchanged from
`crates/mush-sim/src/route.rs`; a `Checkpoint` is `{ id, point: Point3 }` on the corridor.

### 2.4 Travelers, journeys, windows, poses

```rust
pub enum Traveler { Character(ObjId), Object(ObjId), Carrier(ObjId) }
pub struct JourneyRec {
    id: JourneyId, traveler: Traveler, connection: CanonicalId, mode: TravelMode, direction,
    departed: ClockStamp /* WorldIc */, advanced_at: ClockStamp, cursor: u32,
    state: UnderWay | Held { at: Checkpoint, by: HoldReason } | Arrived | Aborted,
    destination: SpatialParent | Room(ObjId), applied_revision: u64, anomaly: Option<String>,
}
pub struct Pose { x: f32, y: f32, z: f32, facing_deg: u16, posture: RulesValue }   // region metres; z per P2
pub struct WindowRec {                      // extends DomainRecord
    domain: DomainId, region, surfaces: Vec<SurfaceRef>, bbox: Rect2, cell_cm: u16,
    composited_from: Vec<(CanonicalId, u64)>, features_seen: u64, opened_by: Scene | Staff,
    revision: u64,
}
```

`DomainRecord` (`crates/mush-sim/src/data.rs`) gains `window: Option<WindowRec>`;
`SpatialParent::Direct { pose }` carries `Pose`, with `posture` the only driver-private part
(D-14-4). `CarrierJourney`'s cursor, `advance_due` and `reanchor` become `JourneyRec`'s
(`crates/mush-sim/src/journey.rs`); `route.rs::Journey` is retired.

### 2.5 Content: packs

A pack is one JSON document: a manifest and entries, digested over canonical JSON by
`pack_digest` (`crates/ruleset-tacsim/src/pack.rs`), lifted with the loader and the seed latch
into `mush-content`.

```json
{
  "manifest": { "id": "neighborhood.staff_lobby", "version": 1, "kind": "neighborhood",
                "digest": "<64 hex>", "requires": [ { "id": "region.limbo", "version": 1 } ] },
  "brief": { "prose": "A film-lot lobby …", "answers": { "q.public_role": "tour" } },
  "entries": {
    "neighborhood": { "kind": "neighborhood", "def": { "region": "region.limbo", "locality": "locality.lot",
                      "name": "Staff Lobby", "setting": "urban", "footprint": [[0,0],[120,0],[120,90],[0,90]],
                      "room_budget": { "target": 7, "maximum": 7 }, "hydration": { "enabled": false } } },
    "site.lobby":   { "kind": "site", "def": { "name": "The Lobby", "site_kind": "site.office",
                      "footprint": [[10,10],[50,10],[50,40],[10,40]], "entrances": ["portal.front_doors"] } },
    "structure.lobby": { "kind": "structure", "def": { "site": "site.lobby", "footprint": [[10,10],[50,10],[50,40],[10,40]],
                      "floors": [ { "index": 0, "z_offset_m": 0 }, { "index": 1, "z_offset_m": 3.6, "name": "Mezzanine (Production Office)" } ] } },
    "room.lobby":   { "kind": "room", "def": { "name": "Lobby", "site": "site.lobby", "surface": { "floor": ["structure.lobby", 0] },
                      "footprint": { "rect": [[12,12],[48,38]] }, "desc": "…", "details": { "desk": "…" } } },
    "room.production_office": { "kind": "room", "def": { "name": "Production Office", "site": "site.lobby", "surface": { "floor": ["structure.lobby", 1] },
                      "footprint": { "rect": [[12,30],[48,38]] }, "access": "restricted" } },
    "exit.stairs":  { "kind": "exit", "def": { "from": "room.lobby", "to": "room.production_office", "names": ["up;u", "down;d"],
                      "portal": { "mode": "stair", "at": [[30,30,0],[30,30,3.6]], "dz_m": 3.6 } } },
    "feature.desk": { "kind": "feature", "def": { "feature_kind": "counter", "surface": { "floor": ["structure.lobby", 0] },
                      "polyline": [[20,20],[30,20]], "z_range": [0, 1.1], "blocks": { "movement": true, "sight": false } } },
    "npc.susie": { "kind": "npc", "def": { "name": "Susie", "tier": "minor", "room": "room.lobby",
                      "profile": { "persona": "…", "fallback": ["The tour starts at the top of the hour."] },
                      "lore": [ { "tier": "public", "fact": "The tour starts at the top of the hour." },
                                { "tier": "guarded", "fact": "The tour is running late again." } ] } },
    "secret.late":  { "kind": "secret", "def": { "truth": "…", "reveal": "staff" } }
  }
}
```

Rules: `entries` keys are the pack-local ids; a reference is `"pack.entry"` only when the pack
is named in `requires`, else bare; a region pack carries `locality`, `connection`, `checkpoint`,
`background` and `heightfield` entries; a `site` addition pack carries the same entry kinds as a
neighborhood pack plus `addition_of`; the `brief` block is optional content, kept so the pack can
be reconceptualized (3.7); tiles are **not** in a pack unless imported — the rasterizer produces
them at apply (3.2). Prose fields are prose; the digest covers them like any field.

### 2.6 The starter area, as content

Two packs ship in the repository and apply at first boot when `[game].starter_area = true`
(default; the testkit's `boot_world` leaves it false). Names and voices are the owner's
(2026-09-09). Most of the names and two of the lines are an homage to a 2004 film about a high
school: per doc 06 §15 the packs' `brief` records that origin, the quotations stay short, and
nothing else is imported.

**`region.limbo`** — one surface region, "the lot", 400 m square, flat at datum 0 with a raised
loading dock (+1.2 m) on the back lot; locality `locality.lot`; background recipe "paved lot";
the connection `connection.lot_tour` (journey; mode `bus`; a corridor from the OOC bus stop past
`checkpoint.security_gate` and `checkpoint.water_tower` to the lobby forecourt). The **Studio Tour
Bus** is a carrier object with a one-room interior (`room.bus`) whose journey loops on that
connection: it is how newcomers arrive at the lobby, and the standard transit test.

**Limbo `#0`** keeps its identity as the spawn room and hub: exits `OOC Area <O>` (open) to the
Welcome Hall and `Staff <S>` (open by default, D-14-27) to the lobby forecourt. `#0` belongs to no
neighborhood.

**`neighborhood.ooc_area`** (six rooms, public, no secrets; hydration policy: one player-built
games room allowed):

| Room | Purpose |
|---|---|
| Welcome Hall | arrival from `<O>`; signposts to the tutorial and the bus |
| Tutorial: Moving | exits, `look`, `home` |
| Tutorial: Talking | `say`, `pose`, `page`, channels |
| Tutorial: Building | a sandbox where `BUILDER`-flagged players may `@dig`; reset by staff |
| Lounge | the OOC hangout; the notice board |
| Bus Stop (OOC) | where the tour bus departs |

**`neighborhood.staff_lobby`** (eight rooms; every NPC room public; one restricted room):

| Room | Surface | Purpose and occupant |
|---|---|---|
| Forecourt & Bus Stop | ground, outdoor | arrival from `<S>` and from the bus; **Regina**, on the lookout for it |
| Lobby | Lobby building, floor 0 | reception; **Susie** |
| Breakroom | Lobby building, floor 0 | the staff breakroom; **Phil** |
| Production Office | Lobby building, floor 1 (mezzanine, +3.6 m) | **restricted** to staff: the one locked room, and the stair portal's `dz` |
| Souvenir Shop | its own site | **Gretchen** |
| Backlot | ground, outdoor; the loading dock | circulation; **Karen Smith**, the meteorologist |
| Coffee Shop | its own site on the back lot | the open house venue; **Cady Heron**, the economist |
| Writers' Room | its own site | **Janis Ian**, the set designer: the builder's front desk |

**NPCs** — all minor tier, enabled, on the default per-NPC budget, each with fallback lines so
the lot works with `[llm].enabled = false`; the interface abilities are §4.6:

- **Susie** — receptionist, Lobby. ENFP: warm, quick, curious about everyone. Greets newcomers
  getting off the studio tour bus. Plays guitar and sings her originals at the back-lot coffee
  shop open house. Happy to tell you the layout of the lot and who's around today (interface
  abilities `atlas.describe`, `who.nearby`). Public lore: the lot's layout, the bus schedule, the
  open house; guarded: the tour is running late again; does not know why. Fallback: "Welcome to
  the lot! The bus just got in — coffee shop's out back, and I'm right here if you get turned
  around."
- **Phil** — off-duty gamemaster, Breakroom. Works at an off-screen comic book shop; has an
  opinion on everything gaming related; loves to brainstorm character concepts (interface
  abilities `chargen.explain`, `rules.lookup`). Public lore: how scenes and `+request` work,
  what a storyteller does; guarded: which storyline he thinks is overrated. Fallback: "I'm off
  the clock. Tell me about your character anyway."
- **Gretchen** — owner of the Souvenir Shop, selling memorabilia that is still trying to make
  fetch happen. Public lore: the shop's stock (props, posters, "fetch" merchandise nobody buys,
  the tutorial pamphlet), the lot's history as she tells it; guarded: everything she has been
  told in confidence, which is why she is told so little. Fallback: "That is so fetch. It's
  going to happen. Can I help you find something?"
- **Karen Smith** — the meteorologist, Backlot. Talks about the weather (`weather.report`,
  `weather.forecast` over the lot's weather cell); her method is not the instruments. Public
  lore: current conditions and the forecast, read from the cell when the model is on; guarded:
  how she really knows it is going to rain. Fallback: "Hi, this is Karen Smith. It's 68 degrees,
  and there's a 30 percent chance that it's already raining."
- **Regina** — Forecourt & Bus Stop, on the lookout for the studio tour bus. She can tell you
  when the next one is due and where it is (`travel.next`, `travel.status` over the bus's
  journey). Public lore: the bus loop, the checkpoints, who got off last; guarded: why she
  watches for it so carefully. Fallback: "The bus? It's coming. It's always coming. …Stand back
  from the curb."
- **Cady Heron** — the economist, Coffee Shop, doing the studio's books at the corner table.
  Shows you the markets (`market.quote`, `market.list` over the public projection) and explains
  what a listing, a quote and a hold are. Public lore: how the market commands work, what is
  cheap this week; guarded: whose books do not add up. Fallback: "The limit does not exist. The
  price, however, is on the board."
- **Janis Ian** — the set designer, Writers' Room: the builder's front desk. Talks a brief into
  a plan (`build.propose`, `build.status`, `build.answer`) and never applies it. Public lore: how
  a neighborhood pack is shaped, what the validator refuses and why; guarded: which sets on the
  lot she thinks should be torn down. Fallback: "Tell me what the place is for. Then tell me what
  it looks like. In that order."
- **Aaron Samuels** — the tour bus's driver, aboard `room.bus`: the carrier's declared NPC
  fallback at its one station (C30A); fronts nothing; drives the bus Regina is watching for
  (owner, 2026-09-09: decided). Fallback: "Next stop, the lobby. Please stay seated."
- **Bench** (owner-supplied names, proposed assignments; no new rooms): **Sharon Norbury** as the
  OOC Area's docent in the Welcome Hall (extra tier; walks newcomers through the tutorial rooms;
  fronts `help`); **Ron Duvall** in the Lobby as the front for filing a request with staff
  (`request.file`, `request.status` over `+request` and `+job`, the asker's own tickets only).

**Hooks**: `hook.open_house` — Susie's set at the coffee shop, an invitation and a place
strangers meet, no mechanic. **Items**: the tutorial pamphlet (readable), the shooting-schedule
clipboard (details), the notice board (details), the lost-and-found box (container), a souvenir
mug. **Secret**: `secret.late` — why the tour is late, stored `Secret`, never in a prompt.

Both packs include their `brief` blocks (3.7). Fourteen rooms plus Limbo; every room is
justified by a tutorial step, an arrival, an NPC or a demonstration (the stair, the dock, the
lock, the bus).

## 3. Behaviour

### 3.1 Boot and apply

1. First boot seeds Limbo `#0` and God `#1` as today (`crates/mush-server/src/persist.rs`), then
   applies the starter packs through the ordinary content path, in `requires` order, with **zero
   LLM calls** (P9). A failure to apply pages staff and leaves the minimal world; it never leaves
   a half-lot.
2. `apply(pack)` = **plan** (parse, digest-check, resolve `requires`, validate — 4.3 — and diff
   against the latch) → **apply** (create / update / retire in one `WriteBatch`, objects and the
   latch together, exactly as `seed::apply` does today) → **latch** (`placed: BTreeMap<CanonicalId,
   ObjId>`, per-entry revisions, the pack digest). Update touches only source-owned fields (names,
   descriptions, geometry, initial profiles and lore); live state (memories, inventory, damage,
   relationships, tenancy, scene history) is never in a pack's diff. Retire is refused while an
   entry's object is occupied, referenced by an exit or a scene anchor, or holds a title (P8).
3. `@atlas/pack list|show|plan|apply|retire`, staff-gated; `plan` prints the diff and the
   validator report; `apply` requires a plan digest to match, so a pack edited between plan and
   apply is re-planned.

### 3.2 Making geometry from a brief: the rasterizer

A pack need not draw. `atlas::raster` turns what a brief naturally states into the substrate:

- a site or structure rectangle → a footprint; a room `rect`/`polygon` → cells on its surface; an
  `anchor { point, radius }` → a disc of cells;
- an exit row → a portal on the shared boundary of its two footprints (the midpoint of the
  shared edge; if the footprints do not touch, the exit is a journey portal and needs a distance);
- a structure's footprint edges → wall features on every floor; a portal of mode `door`/`gate` →
  a door feature; a floor's `z_offset_m` → the floor's height map (flat, plus the ground height at
  the structure's anchor);
- everything else in the neighborhood footprint → the region background at the neighborhood's
  fine resolution, written as tiles only where a room or feature exists, else left implicit.

The result is legal, coarse and playable: the first-slice map of any neighborhood, including the
starter lot, with no drawing and no model. Typed features from the agent (4.2) and imported
heightfields (3.5) refine it later through the same records.

### 3.3 Travel and interception

- **Local exits** are instantaneous unless the portal's `time` is `Timed`; a timed portal is a
  one-leg journey on an implicit connection derived from it (distance, `dz`, mode set) and behaves
  exactly like a corridor journey, including P6.
- **`+travel <destination>[/<mode>]`** lists destinations the character knows (public access
  points of authored neighborhoods plus a per-character `known_places` set that grows by visiting
  or being told), filters modes by what the character has, chooses the route by the connection's
  policy and shows alternatives only where mode or hazard differ; a ride (`foot`, `car`, `transit`)
  needs no object; a carrier journey moves the vehicle and everyone aboard (C16A).
- **Clocks.** A connection's cost policy yields IC seconds; the journey is stamped and advanced on
  `WorldIc` (`World::world_ic_now`); real time elapsed is IC over the campaign rate
  (`[game].ic_ratio`, or `[tacsim.time].rate` for that campaign). Two guards make compression
  safe: `dwell_floor_s` per checkpoint (default 30 real seconds) so a stake-out can act, and
  `real_cap_s` per mode (default 180 real seconds for `foot`/`car`/`transit`; none for `ship`/
  `torch`, whose campaigns set their own) so a city crossing never eats a real hour. A WoD
  campaign that runs IC at four times real time simply arrives sooner; the floor still holds.
- **Checkpoints and stake-outs.** `+stakeout <checkpoint>` (from a room whose neighborhood the
  checkpoint's corridor touches, or from a vehicle) materializes a patch at the checkpoint's
  point from the background (P4) with one implicit room, places the watcher there, and registers
  the watch. A traveler reaching a watched checkpoint is **held** there; the encounter runs as a
  narrative room or, if either side opens one, a window (C20A, `PromotionTrigger::Interception`);
  release continues the journey with `route.rs`'s `RESUME_GRACE_S` anti-grief grace. With no watcher, only the leg's hazard band
  applies through the ruleset's encounter policy. `+intercept` becomes "stop the traveler at my
  checkpoint or in my room".
- **Holds and failures.** Cancel, disconnect, a closed connection or a failed validation leaves
  the traveler at its last committed checkpoint, projected into `in_transit_room` as today; a
  destroyed traveler ends the journey; restart resumes from the cursor (P6, P7).
- **Retired:** `@route` and `route.rs`. `@journey`'s plan/depart/show/divert become
  `+travel` and `@atlas/journey`.

### 3.4 Windows: creation, compositing, write-back

- **Creation.** When a scene asks for a map (a facilitator's `+map/open`, a ruleset's engagement
  start, or staff `@atlas/window`), the window's `bbox` is the union of the scene's anchor-room
  footprints plus a margin, its surfaces the anchors' surfaces plus the ground, its `cell_cm` the
  ruleset-registered size for the engagement kind (street 100 cm; TacSim ground 1000 cm default);
  a window over the cell cap coarsens to the next registered size (D-14-19). One window per scene;
  `MAX_ACTIVE_DOMAINS` bounds the total.
- **Compositing.** Sample every covering tile at `cell_cm` (finer tiles averaged for elevation
  and majority-classed, coarser tiles upsampled), background where none, features overlaid, the
  room index from footprints; pin every tile revision read; store the composite under the
  existing `mod:tactics:map:<domain-id>` key (doc 06 §9.1). Off-loop, one bundle.
- **Placement.** Entering a window from a room: pose = a free cell of the room's footprint on
  the room's surface adjacent to the portal the actor arrived through, else any free cell, else
  refuse; the object is placed `Direct` with a metric `Pose`. Leaving: the room whose footprint
  contains the pose on that surface, else the nearest footprint within 8 cells (ties by id), else
  the domain's `presentation_location` — this is the resolver doc 06 D1 deferred, and `Obj.location`
  follows it (P1). Speech and looking use the resolved room (D-14-18).
- **Growth.** A participant moving off the window's edge extends the window to the neighboring
  footprint's tiles (re-composite, same pins plus new ones), bounded by the cell cap.
- **Write-back.** At tick boundaries a window writes feature states (`Damaged`, `Destroyed`) and
  object poses; never a cell (P5). Closing a window keeps every feature state; the next window over
  the same ground sees the wreckage.

### 3.5 Elevation in narrative and in mechanics

- Narrative: a room's description may state its height and gradient as facts the engine
  supplies (`ground` at the anchor, floor index, `dz` of each portal); `look` at a stair says "up";
  the atlas view shows elevation bands in text (§6).
- Mechanics: slope between two cells comes from their elevations and the cell size; the
  locomotion table (§5) turns slope into cost or impassability; line of sight uses elevation
  along the ray; falling is a ruleset's business.
- Import: a region or neighborhood may reference a heightfield file (16-bit little-endian grid or
  a plain-text grid) in a `heightfield` entry; import rasterizes it to ground tiles at the entry's
  resolution. Not built in the first cuts; the record exists so packs are stable.

### 3.6 Retirement

`@atlas/pack retire <pack>` or `@atlas/site retire <site>`: reference checks (occupants, exits
from elsewhere, scene anchors, titles, tenancies, journeys in flight, pinned windows), a
reviewed confirmation, a canon event, then the latch drops the entries and objects go through the
ordinary destroy path with its gates. Never automatic; never on inactivity.

### 3.7 Reconceptualizing the starter area (D-14-26)

The starter packs carry their `brief` blocks — the prose and the answers that produced them.
`@atlas/propose --from neighborhood.staff_lobby` starts the authoring workflow (§4) seeded with
that brief; the staffer answers differently ("the lot is a spaceport", "the coffee shop is a
noodle bar"), the agent produces a new pack under the same ids where they still fit, the plan
shows the diff against the shipped lot, and apply replaces it under the latch with the tram
connection's endpoints preserved. This is the shipped worked example of the whole workflow, and
it costs nothing until a staffer runs it.

## 4. The building agent

### 4.1 Contract

- **Inputs** (D-14-21): prose (any length), a partial pack, or a complete pack; plus context the
  engine supplies — the region pack, the neighborhood's constraints and hydration policy, the
  site-kind taxonomy, existing sites nearby, the theme in effect, the pack schema.
- **Tools** (D-14-22): `atlas.read` (region, neighborhood, sites, connections — public facts plus
  what the actor may see), `atlas.search_sites`, `pack.propose` / `pack.revise` (typed), `pack.validate`
  (runs 4.3 and returns findings), `ask` (appends questions to the proposal record). No tool
  mutates the world.
- **Loop.** At most 6 rounds; stops early when the pack validates with no open questions. Runs
  off-loop under `LlmFeature::WorldgenPlan` (route `worldgen_plan`), the actor's daily budget
  (`BudgetScope::Actor`), and a per-build ceiling: `[worldgen].plan_cost_cap_usd = 5` (hard: the
  loop stops and the proposal is left resumable) and `plan_cost_target_usd = 2` (told to the
  model as its budget). Every call is audited; the kill switch applies.
- **Outputs.** A complete pack, a validator report, and a question list on a durable **proposal
  record** (`{ id, author, scope, source, revision, pack, findings, questions, answers, status }`),
  answerable in-game (`+build/answer`, `+build/show`) or on the web from the same record
  (D-14-24). Nothing touches the world until `+build/go` (staff, D-14-23) or a player's `+build/go`
  under a hydration policy (4.5).
- **Boundary.** The model emits typed records; it never emits command text (doc 03 §7). The
  validator, not the prompt, is the guarantee (G7).

### 4.2 What the agent may propose

Everything a builder may create: neighborhoods, sites, structures and floors, rooms with
footprints, exits with portals, features, connections' access points on its side, NPC profiles
and lore (public and guarded; a `secret` entry is stored and never enters any prompt), hooks,
items, descriptions and details. Site-kind templates are offered as hints, never imposed
(R1).

### 4.3 The validator

Deterministic, engine-owned, the same for a human's pack and the agent's:

1. **Schema and ids** — canonical ids, resolvable references, `requires` satisfied.
2. **Geometry** — footprints inside the neighborhood; site footprints disjoint unless a shared
   boundary is declared; room footprints disjoint per surface; portals on shared boundaries or
   with explicit distances; floors with consistent offsets; no feature outside its surface.
3. **Topology** — every public room reachable from a public access point through public portals;
   private ingress / free egress where declared; no room without a portal unless declared isolated.
4. **Budgets** — room budget, tile count, features per tile, NPC count, lore sizes.
5. **Policy** — the neighborhood's hydration policy (kinds, reserved land, `Property` footprints,
   no shortcut through private space); the site-kind taxonomy; the theme's banned vocabulary.
6. **Secrets** — no `secret` text in any NPC prompt field (`persona`, `background`, `goals`,
   `fallback`, `knows`); staff-only entries flagged.
7. **Supported definitions** — services, sheets and mechanics named exist in the loaded rulesets;
   unsupported ones are marked staff-run, never invented.
8. **Diff safety** — an update never touches live-state fields; a retire has no live references.

Findings are typed (`{ code, entry, severity, message, fix_hint }`); the agent gets them back as
tool output; a human gets them in the plan.

### 4.4 Staff path

`@atlas/propose <prose | file>` → rounds → questions on the proposal → `+build/go` → apply live.
A post-hoc `BUILD` ticket is opened with the pack digest for the record; nobody has to act on it.

### 4.5 Player path: hydration

1. **Request.** `+build/place <kind> <where>=<prose>` in a neighborhood whose policy enables it.
   The engine reserves, through C8 `ReservationRecord`s in one `ReservationBook`: one slot of the
   character's **outstanding cap**, one unit of the neighborhood's expansion capacity, and the
   footprint or an unclaimed fitting one. The request id is an `IdempotencyKeyText`; retries and
   second tabs converge; two requests for one implied site converge on the first.
2. **Build.** The agent runs with the neighborhood's constraints; the player answers its
   questions; validation must pass.
3. **Activate.** Apply as a `site` addition pack: `origin: Hydrated`, `review: pending_review`,
   `tactical_ready` per 2.1. Playable at once; a canon event records it; a `BUILD` ticket goes to
   the neighborhood's reviewers with the request, pack, validator report and nearby duplicates.
4. **Review.** Approve (records the revision, releases the slot), correct (a scoped update pack
   by the reviewer), request changes (the site stays live; the reviewer may suspend new entry), or
   **strike** (the site is retired under 3.6 and the character's strike count rises). Nothing
   expires.

**Caps and strikes (D-14-25).** `hydration_cap` per character, default `[worldgen].hydration_cap
= 2`, counts sites `pending_review` plus requests generating; `@atlas/cap <character>=<n>` sets
it; a strike is `@atlas/strike <site>=<reason>`; at `[worldgen].hydration_strikes_to_zero = 3`
the cap becomes 0 until staff reset it; staff raise caps for players whose sites keep passing.
All of it is on the character (accounts are not principals), audited, and visible to the player
(`+build/status`).

### 4.6 Interface NPCs (owner direction, decided 2026-09-09; 14b's first slice)

The owner's idea: the staff zone's NPCs as the conversational face of game systems — the builder
as a Set Designer you talk to, an Economist who shows you the markets, a Meteorologist who talks
about the weather, a receptionist who knows the lot and who is around. It fits the tree better
than it first looks.

**What exists.** An NPC agent already acts only through a closed tool list — say, pose, move,
look, remember, use an ability, do nothing (`crates/mush-npc/src/lib.rs`, `NpcAction`, bounded by
`MAX_ROUND_TRIPS` and `MAX_ACTIONS`) — and `UseAbility` is resolved on the world loop by
`Ruleset::invoke_ability` (`crates/mush-ruleset/src/lib.rs`), the NPC's `AgentCapabilityProfile`
deciding which abilities its prompt is offered. `wod_roll` and `sr_roll` are abilities today. Doc
03 §7's rule — the allowlist, not the prompt, is the boundary — is what makes the idea safe.

**The design.** An **interface ability** is an ability whose provider is a game system rather than
a ruleset sheet, registered through the same seam and invoked the same way:

| Ability | Fronts | Backed by |
|---|---|---|
| `atlas.describe(place)`, `who.nearby()` | the receptionist | `+atlas` and the presence projection, filtered for the asker |
| `weather.report(zone)`, `weather.forecast(zone)` | the meteorologist | `mush-weather`'s cell for the lot's zone (the read side of `@weather <zone>`) |
| `travel.next(connection)`, `travel.status(journey)` | the bus-watcher | the journey record and `+travel`'s view of it, filtered for the asker (a traveler's destination stays hidden, as `+intercept` hides it today) |
| `market.quote(…)`, `market.list(…)` | the economist | `+market`'s public projection (`crates/mush-market/src/commands.rs`), never the book beneath it (doc 10 §14's safe projection) |
| `build.propose(brief)`, `build.status(id)`, `build.answer(q, a)` | the set designer | the authoring agent (4.1); the conversation *is* the proposal record's question loop; the NPC never applies |
| `chargen.explain(topic)`, `rules.lookup(topic)` | the off-duty gamemaster | the help topics and the loaded ruleset's chargen schema |
| `request.file(text)`, `request.status()` | the front office (proposed: Ron Duvall) | `+request` and `+job` (`crates/mush-jobs/src/api.rs` `open_job`), the asker's own tickets only |
| `help.topic(name)` | the docent (proposed: Sharon Norbury) | the help topics, verbatim |

Four rules keep an interface NPC exactly as powerful as the person talking to it:

1. **Authority is the asker's.** Every interface ability runs with the speaking character's
   principal, powers and visibility; the NPC holds none of its own. Gretchen cannot tell you a
   hidden faction's holding because a wizard asked her about it yesterday.
2. **Read or propose, never mutate.** An interface ability either reads a projection the asker
   could read with a command, or writes to a proposal record the asker owns. The mutation stays
   a typed command the asker issues (`+build/go`): the NPC drafts, the validator checks, the person
   commits — trust but verify, in a costume.
3. **Cost lands where the work is.** A read is charged to the NPC's own daily budget like any
   turn; an ability that runs the authoring agent is charged to the asker's actor budget under
   `worldgen_plan` with the $5 / $2 ceilings (4.1). Tier and cooldown bound the chatter.
4. **Audited and fallback-safe.** Every call is audited as `(npc, asker, ability, args)`; with
   the LLM off, each interface NPC's fallback lines name the command that does the same thing
   (`+atlas`, `weather`, `+market`, `+build`, `help chargen`).

**Why the staff zone.** It is where the game shows its workings and it is public by design
(D-14-27): a newcomer learns `+atlas` by asking Susie where the coffee shop is; a builder learns
the authoring loop by talking to the Set Designer before typing `@atlas/propose`. The same
abilities can be given to any NPC anywhere later — a city's cab dispatcher is `+travel` with a
voice — with no second mechanism.

**What it costs to build.** A provider trait for interface abilities beside the ruleset's, both
reached through the one `invoke_ability` path; an asker-scoped authority context on the ability
call; and per-ability projections that already exist as commands. It follows cut 3 as the first
slice of 14b.

## 5. What the engine owns and what rulesets register

D-14-10, made concrete. The engine (`mush-atlas`) defines and answers; rulesets declare
definitions that call the answers.

### 5.1 Engine-defined vocabularies

- **Surface classes** (`CanonicalId`, engine-registered, ruleset-extensible): `clear`, `road`,
  `rough`, `woods_light`, `woods_heavy`, `water_shallow`, `water_deep`, `building`, `rubble`,
  `sand`, `ice`, `vacuum_regolith`, …; each with default cover and opacity.
- **Cover classes**: `none`, `partial`, `full`. **Opacity**: `clear`, `partial(n)`, `opaque`.
- **Locomotion classes**: the atoms `MobilityVocabulary::base` already declares in
  `crates/mush-tactics/src/mobility.rs` — `legged`, `wheeled`, `tracked`, `hover`, `rotor`,
  `fixed_wing`, `displacement`, `thrust`, with the tags `amphibious`, `jump`, `vtol`,
  `submersible` — plus `biped` as the pedestrian default; each an engine row in the **locomotion
  table**: `passable(class, locomotion) -> bool`, `cost(class, locomotion, slope) -> Cost`, slope
  limits, water depth limits. A ruleset may add a locomotion or a class; it may not change a
  core row. The vocabulary moves from `mush-tactics` into `mush-atlas` with the table.
- **Media**: the base vocabulary's `ground`, `air`, `water`, `space`; `mush-propagation`'s
  `Medium::{Vacuum, Interior, Surface}` maps onto them (`Vacuum` = `space`, `Surface` = `ground`
  or `air`, `Interior` unchanged) rather than keeping a second list.

### 5.2 Engine-answered queries

```rust
fn ground(region, surface, x, y) -> Elev;                          // tile, else background (P4)
fn cell(window, x, y) -> CellView { class, elev, cover, room, features };
fn move_cost(window, from: Cell, to: Cell, locomotion) -> Option<Cost>;   // None = impassable
fn path(window, from, to, locomotion, budget) -> Option<Path>;      // bounded A* on the composite
fn line_of_sight(window, a: Pose, b: Pose) -> Sight { clear | partial(n) | blocked_by(what) };
fn direct_fire_path(window, from: Pose, to: Pose) -> Trace;         // straight line; first blocker
fn indirect_fire_path(window, from: Pose, to: Pose, apex_m) -> Trace;   // arc over blockers below apex
fn beam_path(window, from: Pose, to: Pose) -> Trace;                // straight, no drop, opacity attenuates
fn hearing(window, from: Pose, to: Pose) -> Attenuation;            // for propagation and NPC wakes
fn resolve_room(window, pose) -> ObjId;                             // 3.4
```

`Trace` lists every cell crossed, the features and units intersected in order, the cover of the
target cell and the elevation profile. `mush-propagation`'s `Geometry { range, occluded }` is
computed from `line_of_sight` and metric range when both ends are in a window, and keeps its
graph-only 0/`ADJACENT_RANGE` answer when they are not — the swap its author reserved.

### 5.3 What rulesets register

Units (`TacticalUnit`: size class, locomotion mode with `nominal_speed_cms`, height), weapons
(delivery `direct | indirect | beam`, ranges, what a `Trace` result does), damage and armor,
initiative, the encounter policy for unwatched hazard bands, the cell sizes their engagements
use, extra surface/locomotion classes, and cost policies for connections (drive classes,
propellant, alignment bands). A WoD or Shadowrun ruleset registers pedestrians and cars and a
100 cm cell; TacSim registers mechs, tanks, aerospace and a 1000 cm cell over the same tiles.

### 5.4 The first two drivers

- **Street** (D-14-2): 100 cm cells, `biped` and `wheeled`, doors and counters, direct fire and
  beams, hearing; enough for a bar, a street, the lot.
- **TacSim ground**: 1000 cm cells (configurable), `legged`, `tracked`, `wheeled`, `hover`,
  `rotor`, indirect fire, the pulse clock from E7 cut 8, the same tiles. No hexes (D-14-4): the
  `TacticsStub`'s `toy_map`, `HexLock` and any `tactics.hex` naming go when the real module lands.

## 6. Rendering: the terminal first

- **One projection.** The server emits a `atlas.map` panel keyframe/delta: a window (or a
  room-scale or neighborhood-scale view) as a grid of cells with class, elevation band, cover,
  feature glyphs and visible units, **already filtered** for the viewer (doc 06 §10: never raw
  truth). The same frame drives the web canvas and the text renderer; the text renderer is
  written first and is the acceptance test (P10).
- **Text form.** One glyph per cell, a legend, the viewer centred, elevation as band shading or
  digits on request, one surface at a time with a floor selector; fits 80×24 with scrolling.
  `+map` in line mode prints it; the `@tui` gains a map screen beside its damage and unit
  screens (`docs/help/tui.md`); the web draws it on a canvas.

```text
 Back Lot (ground, 1 m)                    legend: . clear  = road  # wall  + door
 ###########+####                                  ~ water  T tree  D dock(+1.2)  @ you
 #..............#........TT.......                 o other  x rubble
 #....@....o....+=======================
 #..............#........DDDD......~~~~~
 ################........DDDD.....~~~~~~
```

- **Atlas views.** `+atlas` shows the neighborhood: sites, rooms, portals, access points and
  connections the character knows, as a text map at 5–10 m and as a list; `+atlas/region` shows
  neighborhood footprints and corridors; nothing the character does not know is drawn.

## 7. Build order and acceptance

The build plan is [14-spatial-implementation-plan.md](14-spatial-implementation-plan.md); it
splits cut 2 in two and adds the interface NPCs as their own cut, keeping this order.

| # | Cut | Delivers | Proves (starter area as the fixture) | Depends on |
|---|---|---|---|---|
| 1 | `mush-content` + `mush-atlas` records | pack loader and latch lifted from `ruleset-tacsim` with update/retire; the `atlas` family; the rasterizer; `@atlas/pack`; first-boot starter packs; `+atlas` text views | both packs import in either order; unchanged reapply is a no-op; retire with references refused; boot makes no LLM call; the testkit stays minimal (P4, P8, P9, P10 for the atlas view) | nothing new |
| 2 | One travel system | `JourneyRec` over connections; `+travel`, `+stakeout`, holds; timed portals; the tram; `route.rs` retired; theme and weather on the atlas (D-14-28) | the tram carries passengers with one location each; a stake-out sees it; restart resumes at the checkpoint; the farm-road pattern as a timed portal; a WoD rate of 4 arrives sooner but the dwell floor holds (P1, P6, P7) | 1 |
| 3 | The agent and hydration | `worldgen_plan` loop with the validator tool; proposal records; staff path; player path with caps and strikes; reconceptualize-from-brief | a prose brief becomes a valid pack within $5; a trash submission is struck and the cap falls to 0; a second request for the same implied site converges; the lot is reconceptualized from its brief (P8) | 1 |
| 4 | Windows and the street driver | tiles, features, floors, composite, metric pose, resolver, write-back, `atlas.map` text and web, `+map` | a bar fight and a back-lot fight in windows; a fight that spills into the street grows the window; a broken counter survives a description rebuild; the mezzanine's `z`; every frame renders in text (P2, P3, P5, P10) | 1, 2 |
| 5 | Engine physics | locomotion table, LOS, traces, hearing; `mush-propagation` on the metric | biped vs wheeled costs; a wall blocks sight and a window does not; an indirect shot clears a wall a direct one hits | 4 |
| 6 | TacSim ground driver and imports | 1000 cm windows, tracked/legged/rotor, indirect fire, heightfield import; hex vocabulary removed from the tactics stub | a mech engagement on imported relief over the same tiles a street fight used | 5, E7 cut 8 |

Interface NPCs (§4.6) follow cut 3 as the first slice of 14b (D-14-32).

Acceptance scenarios beyond the table: a landing at an implicit point yields a bounded patch, a
perimeter and one arrival room and persists nothing until a feature is damaged there; a ship
departs a pad on a cross-frame connection, is held in orbit, and lands elsewhere with every
occupant's location intact; a guarded fact reaches a prompt with its framing and a secret never
does, from a hydrated NPC exactly as from a staff-authored one; a window over two neighborhoods
pins both fine tile sets and the background; a hyphenated id is refused at load naming the entry.

## 8. What this document supersedes

- **Doc 03** §5.2–5.3 "never invents topology" and §10 item 5 — by R1 and §4 here.
- **Doc 04** "hex maps with terrain types and elevation per hex" — square cells (D-14-4).
- **Doc 06** §3.3 "terrain-cost and kinematics maths to the ruleset" and `ruleset-tacsim`'s
  "hex-map terrain and coordinate data" — by §5 (engine-owned); §5.1 opaque pose — metric `Pose`
  with driver-private `posture`; §6.1 `"tactics.hex"` and §10's "N-hex radius" / "hex-by-hex" —
  square windows and the text projection; D1's deferred `PresentationResolver` — 3.4's resolver.
- **Doc 07** §9 — `MobilityJob` layers on `JourneyRec`; its checkpoint property is P6.
- **Doc 10** §2 and 1A/6A/C16A/C19A/C20A — kept; §16 item 9 (carrier journeys) — done under 2.4.
- **Doc 11** — §2 vocabulary kept; §3 coordinates → §2 here; §4 travel → 3.3; §5 tactical → 3.4
  and §5; §7 Stage B → §4; §8 hydration → 4.5; §10 seam table and build order → §7. §§6 and 9
  remain doc 11's until 14b/14c. Doc 11 is marked superseded where 14 differs.
- **Help topics** `worldgen`, `route`, `spatial`, `tui` — revised when their cuts land; until
  then they describe the shipped commands truthfully.

## 9. Open items

- The exact packed cell encoding and tile document size against the store's document limits
  (cut 1 measures).
- Whether `known_places` lives on the character record or in the epistemic layer when that
  lands (cut 2 chooses the record; the layer may absorb it).
- The pedestrian encounter policy for unwatched hazard bands in city campaigns (a ruleset
  decision; the engine only exposes the band).
- The bench's remaining proposed assignments (Norbury, Duvall) await the owner's nod; every
  named role is cast and Aaron drives the bus.
