# 30. World generator v2 — the ground as content, one pipeline from authoring to the three views

**Draft 0.2 — 2026-10-30, the review folded.** The author of PR #178 reviewed Draft 0.1 on PR
#183 and raised six points, all taken (the owner, the same day: *"fold all six points into doc 30
as Draft 0.2 … and rule Q-30-8 as the reviewer suggests pending my confirmation"*): the writer's
repair passes no longer touch a studio draft by default and never a built class (§5); the draft's
`classes` encoding is spelled out — the atlas's three-byte class runs, not PR #178's eight-byte
cells (§4); the waypoint graph is authored in the studio or routed by the writer (§4, §5, §8); the
studio gains a viewshed overlay and runs the §5 report live, with a fixture test holding its
TypeScript to the Rust check (§8, §11, §14); and `rock` is ruled a surface, not a block —
**D-30-10** in §7, at the reviewer's default, confirmed by the owner the same day — because the
sim's sight table makes only `wall` opaque and the woods partial, while the picture had been
raising a wall out of it. The cut rows WG.2–WG.4 carry the work.

**Draft 0.1 — 2026-10-30.** Based on `origin/main` at `a1ddefaa`. The owner's ask, verbatim:
*"since the feedback is more general, rather than comment on the PR let's write a world generator
v2 design document ... this sort of just grew organically one commit into the next without any
pre-planning so it's a good time to step back and treat this as a research spike and now write the
proper doc."* Earlier the same week, of the Unity extractor: *"serves as an initial example while
we plan and upgrade our own map generator."*

This document reads the week's terrain work — PR #154 (the 2.5D retained compositor), the Unity
terrain extractor (an operator pack, not in the tree), PR #169 (biome palettes and client relief),
PR #173 (`ArenaDef.biome` and load-time relief) and PR #178 (the web Terrain & Arena Map Studio) —
as **one research spike**, records what each piece taught, and decides the pipeline the spike was
groping towards: where the ground is authored, what the one truth is, how it reaches the sim, the
wire and the three pictures, and what the three authoring surfaces (the generator, the extractor,
the studio) have in common. Decisions are **D-30-*n***, questions for the owner **Q-30-*n*** with
the default the lead takes until ruled, cuts **WG.*n***. It amends doc 16 §3.2/§4, doc 21 §7–§8
and doc 14's region record (§12), and it disposes of the spike's PRs one by one (§10).

## 1. The spike, read as research

What the tree holds for the ground today, before any of this:

- **The truth of the ground is a signed atlas pack** (doc 14 §2.2; doc 16 §4 "Content packs"; 16b
  cut C.2). `crates/tacsim/content/atlas/proving_ground.json` is the atlas pack
  `tacsim.proving_ground` (version 3): a `region` record (`region.proving_ground`, `body` `luna`,
  extent 20 000–22 560 m, a zero-relief `background` recipe), a `locality`, and four `heightfield`
  quadrants `field.q{x}_{y}` of 128 × 128 cells at 10 m, each carrying `le16` elevations and the
  run-length `classes` grid (`HeightfieldDef`, `crates/mush-atlas/src/def.rs:818`; a quadrant
  because `MAX_ENTRY_BYTES` is 64 KiB). `crates/tacsim/content/arenas/proving_ground.json` is the
  campaign-seed pack whose `arena` record names that region and those four tiles, the 256 × 256
  window at `origin_m` 20 000, the two entry zones, five key spots and the waypoint graph
  (`ArenaDef`, `crates/tacsim/src/definitions/arena.rs:207`, `deny_unknown_fields`).
- **The arena loads only from the pack.** `from_pack` (`arena.rs:929`) refuses a tile in another
  region or at another resolution, requires every cell covered, and prices each cell from the
  shipped locomotion table. Nothing is invented at load.
- **The generator is the authoring source.** `crates/tacsim/tests/support/ground.rs` writes that
  pack (`atlas_pack_with(Relief::V3)`: the class grid, the V3 landscape — Signal Hill about 100 m,
  the bowl, the flat corridor rows 86–108 kept for the playtest — and `sign`), and
  `crates/tacsim/tests/shard_2/content.rs` pins the shipped file to it byte for byte
  (`TACSIM_WRITE_CONTENT=1` rewrites and re-signs; CI asserts). A hand edit is a failing test.
- **The sim reads relief for real.** `crates/tacsim/src/arena/slope.rs` composes the atlas's
  `slope_pct` with the locomotion table's `max_slope_pct` (symmetric, so a unit can leave the hole it
  entered); `horizon.rs` builds the line-of-sight index over `elev_cm` with `EYE_HEIGHT_CM` 150;
  `path.rs` and `perception.rs` read both.
- **The wire carries relief once.** `arena.map` (doc 21 cut G.7b; `match_/cockpit/frames.rs:908`)
  carries `elev` as zigzag-LEB128 deltas, `unit_cm` 1, base64 — about a byte a cell for rolling
  ground, two for steep — under `MAP_MAX_BYTES` 256 KiB, and the frame's own comment anticipates a
  `window` parameter for the day a map outgrows that (doc 16 §11's 10 km path).
- **The pictures draw what the frame says** (D-21-14): the first-person heightfield (doc 21 §8), the
  overhead's relief shading, and now PR #154's 2.5D canvas all build from `elev`.

The spike added five pieces on top. Read as research, each taught one thing and over-reached in one
direction:

| Piece | State on 2026-10-30 | What it taught | Where it over-reached |
|---|---|---|---|
| PR #154, the 2.5D retained compositor | merged `4e2f52a9` (2026-10-30) | A picture can hold a terrain bake larger than the viewport and move the viewport over it; moving costs 27 KB/s against the first-person path's 100 KB/s, idle costs nothing. | No `p=` placement ids (a 4-unit cap, four copies of every sheet); a 1.5× canvas that rebakes every 28 m. Reviewed on the PR 2026-10-29; the `p=` fix is in w11term `074a20c1`. |
| The Unity extractor (`build_unity_proving_ground.py`, the Antigravity checkout) | an operator pack, `tacsim.proving_ground` version 4, not in the tree | Real heightfields produce a playable, signed pack **through the existing route with no engine change**: the operator shelf already lets a higher-versioned external pack win (`mush-atlas/src/module.rs:323`). Relief −2.5…+47 m, slopes relaxed to ≤ 35 % off-road and ≤ 20 % on roads, 60 953 of 65 536 cells tracked-reachable, a 179.6 KB map feed under the 256 KiB cap. | Its class map is speckled (single-cell water and woods islands, each a sim cell with its own cost and opacity); its flood fill is its own verdict, not the sim's; its passes live in a script outside the tree, so no other source gets them. |
| PR #169, biome palettes + client relief | merged `4b4e3a69` | One `rgb_for_biome` table threaded through the fog, sky and horizon tables gives four biomes across all three views, under the 256-colour budget, with a distinctness test. Keep it. | `Terrain::synthesize_relief`: the **picture invents hills** whenever the feed lacks `elev` or carries an all-zero field for a non-temperate biome. That is the one thing D-21-14 forbids; the sim's horizon and slope would disagree with what the pilot sees. `Biome::from_name` sniffs substrings ("ice" in "practice", "crater", "moon"). |
| PR #173, `ArenaDef.biome` / `procedural_relief` | merged `35756c0b` | Relief belongs on the sim side of the wire, bounded, deterministic, flat in the entry zones, scaled down on roads: the right *shape* of rule. | The wrong **layer** (load time mutates a signed pack's classes and heights: water becomes ground on Mars and the ford vanishes while "The Ford" stays a key spot) and the wrong **amplitude**: the "45 cm a cell, the 4.5 % tank grade limit" misreads the table — tracked is 45 **per cent**, 450 cm a cell. Its integer-sine relief disagrees with PR #169's value noise by construction. `biome_variant` has no caller outside its unit test; no biome arena ships. |
| PR #178, the web studio | merged `feddf4f4` | The brush engine (footprints, Bresenham, flood fill, 35 undo levels) and the `mush_atlas::Cells` run-length codec are correct, DOM-free and tested; the three-view wasm preview is the right idea. | Both ends are wired to nothing: `exportArenaDefJson` emits `tile`, `red_muster`, `spots`… which `ArenaDef` refuses, and lacks `pack`, `region`, `tiles`, `entry_zones`, `waypoints`, `tanks`; import needs `tile`, so the shipped arena cannot be opened. The preview feed carries heights as `elev_field` (a character a cell) that nothing reads, so the preview falls through to PR #169's invented relief; its cockpit feed lacks `x_cm`/`y_cm`/`heading_mdeg`, so the camera pin does not move the camera; "send to game" pre-fills `@atlas/window open`, a staff street window, not a load. The 45 cm clamp and the −5…+12 m band make Signal Hill unrepresentable; presets are 32/64/96 cells against a 256-cell arena. |

The pattern across the five: **four authoring sources and three relief generators, none of which
agree, and only one of which (the extractor) actually reaches the sim — through the route that was
there all along.** The spike's real finding is that the route works and that every piece that tried
to shortcut it (invent at load, invent in the picture, export a shape nobody loads) broke the one
rule the arena was built on: the ground is content, signed, loaded, read.

## 2. What stands, what this document decides

Standing, not re-decided here:

- **Doc 14 §2.2** the tile: `class u8, elev_cm i16, cover u8, room u16`, run-length coded,
  base64; `HeightfieldDef` with `le16` samples and the class grid; 64 KiB an entry.
- **Doc 16 §3.2** the eighteen `TerrainClass`es and the locomotion table (`locomotion.json`: biped
  60 %, legged 70 %, wheeled 25 %, tracked 45 %, hover 20 %, displacement 10 %; `climb_cm` 150/300
  for the walkers) as the one pricing of ground; **§3.6** perception over the LOS index; **§4** the
  arena record and the signed pack route; **16b cut C.2** the content pin.
- **Doc 21 D-21-14** the camera on a leash: the picture draws what the frame says. **D-21-9**
  the clock and the hash; **D-21-10** placement (to be amended by #154's second kitty mode, owed on
  that PR); **§5** the budget; **§8** the heightfield; **§13** no textured frames.
- **The owner's rulings of 2026-10-29**: assets baked from the purchased Unity packs may ship, the
  source models never do, the bake scripts ship, and every shipped artifact must be reproducible
  from a committed script plus a named pack; the generator stays the authoring direction; the Unity
  extraction is an interim example.
- **PR #154's review** (on the PR, 2026-10-29): `p=` addressing, one sheet per model, terrain
  tiles, a bake script for `looks/iso25d_mips.bin`, the doc 21 amendment. Not repeated here; WG.6
  schedules it.

Decided here: the principle (§3), the pipeline and the draft format (§4), relief and passability
(§5), biomes (§6), the wire and the pictures (§7, with D-30-10 on `rock`), the studio (§8), the generator and the extractor
(§9), the disposition of the spike's PRs (§10), cuts (§11), amendments (§12), questions (§13).

## 3. The principle: the ground is content — D-30-1

**One truth, one route.** The ground of an arena is the signed atlas pack's heightfield tiles —
classes and elevations — and nothing downstream adds to or alters it. The sim's `ArenaTerrain` is
read from it; the wire's `arena.map` is written from the sim's; the three pictures and the studio's
preview are drawn from the wire's. A layer that invents ground is a bug by definition, whatever the
intent: not `from_pack` (PR #173's remap and synthesis), not `mush-pixel` (PR #169's
`synthesize_relief` and the all-zero fallback), not the studio's preview (PR #178's fallthrough).

Procedural generation is not banned — it is *moved*: it happens in a **generator that emits a
pack**, before signing, where its output is reviewed as content (the pin) and loaded like any other.
The atlas's own `BackgroundRecipe` (`record.rs:241`: a seeded, bounded relief under a region's
untiled ground) is the model — a recipe the region declares, composited once, never a renderer's
guess.

Corollaries:

1. `ArenaDef.procedural_relief` and `apply_biome_and_relief` are removed; `ArenaDef.biome` moves to
   the region (§6). They were merged on 2026-10-30 with no content behind them, so nothing shipped
   changes.
2. `Terrain::synthesize_relief` and the `all_flat && procedural_wanted` branch leave `mush-pixel`.
   A feed with no `elev` draws flat, as it did before G.7b.
3. The studio previews through the **same feed shape the server writes** (§8). What the builder
   sees is what the sim will see.

## 4. The pipeline and the draft — D-30-2, D-30-3

**D-30-2 — one writer.** Every authoring source emits the same intermediate, a *ground draft*, and
one Rust writer turns a draft into the two signed packs the arena needs (the atlas pack with its
region, locality and heightfield quadrants; the campaign-seed pack with the arena record), running
the same repair and check passes for all of them. The writer is `tacsim::ground` with a binary
`ground-pack` (Q-30-1):

```
ground-pack build  <draft.json> --out <dir>      the two signed packs (+ the pin's digests)
ground-pack check  <draft.json>                  size, grade, passability, speckle: the report, §5
ground-pack render <draft.json> --out <png>      the overhead and the three views at the key spots
ground-pack route  <draft.json>                  a waypoint graph over the cost grid when the draft's is empty (§5)
ground-pack gen    <preset> --seed N --biome B   the generator (§9) → a draft
```

The generator moves out of `tests/support/ground.rs` into `tacsim::ground::generate`, and the
content pin keeps asserting the shipped version-3 bytes are the generator's output — the move is
byte-neutral by construction, and that test is its acceptance. The extractor (§9) and the studio
(§8) emit drafts and never packs.

**D-30-3 — `ground.draft.v1`.** One JSON document, under `MAX_PACK_BYTES`, the writer's only input:

```jsonc
{
  "schema": "ground.draft.v1",
  "id": "proving_ground",              // slug: region.<id>, field.<id>.q{x}_{y}, tacsim.arena.<id>
  "label": "The Proving Ground",
  "biome": "temperate",                // §6; the region carries it
  "body": "luna",                      // the astronomy identity, independent of the biome
  "origin_m": [20000, 20000],
  "cell_cm": 1000,
  "cols": 256, "rows": 256,
  "classes": "<base64>",               // the atlas's class runs: [cols u16][rows u16] + [count u16][class u8], LE — below
  "elev": "<base64>",                  // le16, scale_cm 1, datum_cm 0 — HeightfieldDef.data
  "entry_zones": [ { "team": "red", "x0": 0, "y0": 0, "x1": 15, "y1": 255 }, … ],
  "key_spots":   [ { "name": "The Ford", "x": 64, "y": 97 }, … ],
  "waypoints":   { "nodes": [ … ], "edges": [ … ] },   // may be empty: `ground-pack route` fills it (§5)
  "tanks": [ … ],                      // optional; the shipped roster by default
  "provenance": { "source": "generator|extractor|studio", "version": "…", "seed": 42,
                  "inputs": [ "Winter Pack 1 demo valley 2049²", "…" ] },
  "note": "why these numbers"
}
```

The grid is whole in the draft (one entry is only bounded in the *pack*); the writer quadrants it
into ≤ 64 KiB heightfield entries exactly as the generator does today, names them from `id`, and
writes the arena record's `tiles` list from the quadrants (≤ `MAX_ARENA_TILES` 16, so a draft is at
most 512 × 512 at 128-cell quadrants — Q-30-6). `provenance` is the licensing ruling's corollary made
mechanical: a draft from the extractor names the pack and version it read; the writer refuses a
draft with none. `classes` and `elev` reuse the two encodings the atlas already has — the draft
introduces no third.

**The `classes` bytes, exactly** (Draft 0.2, the reviewer's fourth point). The atlas has two
run-length forms, and the draft uses the *second*. `Cells::encode` (`cells.rs`) writes eight-byte
runs — `count u16, class u8, elev_cm i16, cover u8, room u16` — and is what PR #178's TypeScript
codec implements. `HeightfieldDef.classes` (`def.rs:840`) writes a four-byte prefix
`[cols u16][rows u16]` and then **three-byte runs `[count u16][class u8]`**, all little-endian, the
southern row first, base64 over the whole. The draft's `classes` is the latter — what the writer
re-runs per quadrant into each heightfield entry's `classes` key — and `cover` and `room` do not
exist in an arena heightfield. Code `0` (`Void`) means *not authored* (the region's background
recipe fills it), so a draft that means open ground writes `ground`, never `0`. The `elev` bytes
are `HeightfieldDef.data`'s `le16`: `cols × rows` little-endian `i16` samples in the same row order,
`scale_cm` 1, `datum_cm` 0. WG.4 replaces the studio's eight-byte codec with these two.

**The draft is the source; the pack is the build (D-30-3's pin).** Every shipped arena checks its
draft in beside its packs — `crates/tacsim/content/ground/<id>.draft.json` — and the content pin
grows from "the shipped atlas pack is the generator's output" to **"every shipped pack is the
writer's output over its shipped draft"**, in two halves:

1. *draft → packs*, for every arena: `ground-pack build` over the checked-in draft reproduces the
   shipped atlas and campaign-seed packs byte for byte, digests included. This is the half a studio
   or extractor arena has: its draft is the hand-made (or extracted) source, and the packs are
   derived.
2. *preset → draft*, for a generator arena only: `ground-pack gen <preset> --seed N` reproduces the
   checked-in draft byte for byte, so the proving ground stays what it is today — reproducible from
   code, not just from a file.

`the_shipped_atlas_pack_is_this_generators_output` becomes the two tests this names, and
`TACSIM_WRITE_CONTENT=1` regenerates draft and packs together. A hand edit to a *pack* is a failing
test, as now; a hand edit to a *draft* is a reviewed diff of the draft and the regenerated packs,
which is what a map change should look like in review. The proving ground's draft is about 175 KB
(a le16 grid of 65 536 cells in base64, plus the class runs) — the size of the pack it replaces as
the reviewed artifact, since the packs become build output. This is also the licensing ruling's
corollary in its final form: a Unity-derived arena is reproducible from the committed extractor
script plus the named pack *and* its checked-in draft, and a studio arena from its draft alone.

## 5. Relief, grade and passability — D-30-4

**The sim's limits are the only limits.** No generator, extractor or editor clamps elevation to a
number of its own. The grade a cell may present is the locomotion table's `max_slope_pct` per drive
(tracked 45 %, wheeled 25 %, walkers 60–70 %), read through `tacsim::arena::slope` — the same
function the arena refuses a step with — and the check is a **report**, not a clamp.

What relief is *for*, so the generators aim at it (the numbers the arena already carries):

| Feature | Size | Why |
|---|---|---|
| Hull-down lip | 2–3 m rise over 1–2 cells (20–30 % grade) | hides a hull (≈ 3 m) from a 1.5 m eye on the far side; within tracked's 45 % |
| LOS-cutting crest | ≥ 4.5 m above the ground 50–100 m either side | eye height plus a hull, so the LOS index marks the far slope dead |
| A hill worth holding | 30–100 m over 300–600 m (the V3 Signal Hill is ≈ 100 m) | ranges, the horizon tape, a reason to move |
| A trough or wadi | 3–6 m below the plain, 20–40 m wide | cover for a column; the river's job on a dry world |
| The ceiling | the `i16` cell range, ± 327 m | doc 14's cell; the region validator already refuses a datum that leaves it |

Under PR #173's 45 cm a cell, a 12 m rise needs 270 m of run and none of the first four exist. Under
the sim's limits all five do, and the wire pays two bytes a cell on the steep ones.

**The passability report** (`ground-pack check`, and the studio's overlay in TS from the same table):
for each of tracked, wheeled and legged, the fraction of land cells reachable from each entry zone
by the sim's own slope rule; the mutual reachability of the two entry zones; the reachability of
every key spot and every waypoint node; the longest road grade. **Acceptance defaults** (Q-30-3,
Q-30-4): tracked ≥ 90 % of land cells from both zones, legged ≥ 95 %, the two zones mutually
reachable by all three drives, every key spot wheeled-reachable (roads carry wheels, so roads are
authored ≤ 20 %), and every cell of every entry zone within ± 50 cm of its neighbours (flat musters,
as #173 wanted). A draft that fails is a draft, not content; `build` refuses it unless `--force`,
and the report goes in the pack's `note`.

**The repair passes** live in the writer so every source can have them, and (Draft 0.2, the
reviewer's first point) they run **by default on extractor drafts only**. A studio draft is what
the builder sculpted: `build` does not alter it unless `--repair` is passed, because a cliff over
70 % that channels movement or a one-cell `wall` pillar for cover is a decision, not a defect
(D-30-1's third corollary); the generator authors clean grids by construction and needs neither.
The two passes: the extractor's **slope relaxation** (iterated, pulling the higher cell down toward
the drive limit, roads to 20 %, never touching an entry zone's flatness) and the **speckle filter**
(a class region under *n* cells — default 3 — takes its majority neighbour, water and woods
included, so a sim cell is never an island of one). The speckle filter **never removes a built
class** — `paved`, `road`, `floor`, `door`, `platform`, the atlas's `is_built` — nor `wall`, nor a
key spot's cell: a gate or a small building is small on purpose. The studio offers both passes as
tools that show the changed cells on the canvas and in the preview before a save (§8), which is
how a studio draft gets them.

**The route step** (Draft 0.2, the reviewer's second point). `ArenaDef::check` allows an empty
waypoint graph (at most 256 nodes and 1 024 edges), and the stock mind patrols whatever graph there
is, so an arena with none is legal and poor. `ground-pack route` — run by `build` when a draft's
graph is empty — places a node at each entry zone's centre and at each key spot, connects them by
A* over the tracked cost grid under the sim's slope rule with a node every 20 cells along each
path, and writes the graph into the draft, so it is reviewed like the rest. A draft that carries
its own graph is left alone; the report above still checks every node's reachability.

## 6. Biomes — D-30-5

**A biome is a property of the region, carried on the wire, never sniffed.** `RegionDef` gains
`biome: Option<String>` (default `temperate`; the set `temperate | mars | lunar | ice` as #173's
`BIOMES`), validated in `RegionRec::validate`, beside `body` and `theme` — it is not the body (the
shipped proving ground is a temperate valley whose region says `luna`; the astronomy identity and
the ground vocabulary are different facts, Q-30-2) and not the worldgen theme (doc 14 D-14-28's prose
style). `arena.map` emits `"biome"` from the region record; `mush-pixel` reads that word exactly
(`Biome::from_name` becomes an exact match on the four words; the substring sniffing goes) and keys
PR #169's palette, sky and horizon tables on it. `ArenaDef.biome` is removed.

**Classes are authored per biome, not remapped.** A Mars draft contains no grass to remap: the
generator's vocabulary per biome is

| Biome | Ground | Cover and blocks | Water |
|---|---|---|---|
| temperate | grass, ground, road, paved | woods_light, woods_heavy, wall, rubble, rock | water, water_deep |
| mars | sand, regolith, ground, road | rock, rubble, wall | none; a trough is `ground` |
| lunar | regolith, ground, paved | rock, rubble, wall | none |
| ice | ice, ground (moraine), road | rock, rubble, wall | water_deep (leads) |

Woods on Mars or the Moon are not authored; PR #169's "rock spire" drawing of the woods classes
there is retired with the remap (and `rock` is a surface with a cost, not a block — D-30-10). The
pricing is the locomotion table's, per class, unchanged: `ice`
and `regolith` cost what `locomotion.json` says, so a biome changes what is on the ground, never what
the ground costs.

## 7. The wire and the pictures — D-30-6, D-30-7

**D-30-6 — one elevation encoding, and the picture never invents.** `arena.map.elev` (zigzag-LEB128
deltas, `unit_cm` 1) stays the one way heights travel: to the SSH pictures, to the web, to the
studio's preview. `elev_field` (PR #178's string) is not added. The camera reads what it reads
(`x_cm`, `y_cm`, `heading_mdeg`; `cockpit::camera`), so any feed that wants to drive a picture
writes those. The palettes (PR #169) stay; `synthesize_relief` and the all-zero fallback go, and the
contract test `pixel_relief.rs` gains the case "a flat field draws flat under every biome".

**D-30-7 — the cap and the window.** A 256 × 256 map with real relief is near `MAP_MAX_BYTES`
(the Unity v4 feed is 179.6 KB of 256 KiB): `ground-pack check` reports the frame's size and refuses
a draft that would exceed the cap. Past 256 cells, or when a biome's relief costs two bytes a cell
across the whole grid, the frame carries the `window` the G.7b comment anticipated — a slice around
the pilot of the same shape, rebaked by the pictures when it moves — and PR #154's terrain tiles are
the same mechanism on the picture side. The window is its own cut (WG.6), scheduled with the tiling;
until then the cap is a check, and the proving ground (65 536 cells, V3 relief) passes it today.

**D-30-10 — `rock` is a surface, not a block** (Draft 0.2; Q-30-8 ruled at the reviewer's
default, confirmed by the owner 2026-10-30). The sim's sight table
(`crates/mush-atlas/src/physics.rs:153`) makes `void` and `wall` opaque and the woods partial
(25 % light, 60 % heavy); every other class, `rock` included, is clear and has no height — a ridge
is `elev_cm`. `rock` is priced as costly passable ground (biped 200, legged 150, tracked 300, hover
300; wheeled impassable). The picture, though, maps the legend words `rock`, `cliff` and `building`
to its `Wall` ground (`raycast.rs::from_legend_name`) and raises an opaque block of
`WALL_HEIGHT_CM` on it — geometry the sim does not have, which is what D-30-1 forbids. Ruled:
`rock` is a **surface material** on the real heightfield — its cost, its shading, zero extrusion —
and vertical geometry and sight blocking come from `elev_cm` and `wall` alone; the picture raises
only the classes the sim's sight table raises (`wall`; the woods, partially). This also retires PR
#169's rock-spire drawing of the woods classes on Mars and the Moon (§6). The former alternative, a
`rock`/`cliff` split, is not needed: a cliff is a steep `elev_cm` edge or a `wall`. WG.2 carries it.

## 8. The studio — D-30-8

The web Terrain & Arena Map Studio is a **draft editor**: it opens and saves `ground.draft.v1`,
including the shipped proving ground — the server serves the checked-in draft of any shipped arena
(§4's `content/ground/<id>.draft.json`) and, for an operator pack that shipped without one,
`ground-pack`'s inverse, pack → draft; it previews through `mush-pixel-wasm` with the
real feed (`elev` encoded in TS — twenty lines — and the real pose keys); its grade tool is the
passability overlay of §5 drawn from the locomotion table shipped to the web as JSON, with an
optional "relax to passable" brush using the same limits; its height band is the cell's `i16`
range with a slider default of ± 150 m; its canvas presets are 64, 128 and **256** (the arena);
its export is the draft and its route to the server is, first, download → `ground-pack build` →
`[tacsim].pack_paths`/`[atlas].pack_paths` and restart (the operator route that already works), and
later a staff `@ground/stage <draft>` that signs and shelves a draft as an operator pack without a
restart (Q-30-5; the hot-load half is new). The `@atlas/window open` pre-fill goes. The studio's
TS codec stays and gains the `le16` and zigzag encoders; PR #178's engine, undo and Svelte shell are
kept as they are.

**Editing a shipped arena is editing its draft.** Under §4's pin the studio's save of the proving
ground is a new `content/ground/proving_ground.draft.json`, and the change reaches the tree as a
pull request carrying the draft, the packs `ground-pack build` regenerated from it, and the pin's
new digests — the same shape of diff a generator change makes today, reviewable on the draft's
overhead render rather than on 175 KB of base64. A shipped arena that began as a generator preset
and is then hand-edited in the studio loses its *preset → draft* half of the pin (it is no longer
the preset's output) and keeps the *draft → packs* half; the draft's `provenance` records the preset
and seed it started from. Unshipped work stays in the drafts tab; an operator's own arena goes out
through the pack route with no pin at all.

**The studio's tools, after the review (Draft 0.2).** Four additions, each one of the reviewer's
points made concrete:

- **A waypoint layer** beside the key spots: place and drag nodes and draw edges on the canvas; an
  *Auto-route* button runs the §5 route rule in TypeScript over the same cost grid for the
  preview, and `build` runs the Rust one when the saved draft's graph is empty.
- **A viewshed overlay** from the camera pin: the sim's 1.5 m eye (`EYE_HEIGHT_CM`) against a hull
  of the look's height (`size_cm`; 250–300 cm for the shipped units), shading each cell *seen*,
  *hull-down* (the hull hidden, a turret seen) or *dead ground*, with a fourth shade where woods
  cover the line partially (25 % / 60 %). It is a TypeScript walk of the ray `horizon.rs` walks;
  `ground-pack render --viewshed <x>,<y>` draws the same picture from the real index, and the two
  agree on the fixture drafts (§11).
- **The §5 report, live**: reachability per drive from each entry zone, the two zones' mutual
  reachability, every key spot's and node's, the longest road grade, and the `arena.map` frame's
  exact byte count against the 256 KiB cap (free: the studio already builds the exact wire feed
  for its preview) — in the sidebar, green or red, before a download. `ground-pack check` stays
  the verdict; the TypeScript report is held to it by a fixture test (§14).
- **Relax slope** and **Despeckle** as tools: the §5 passes run in the browser on demand, the
  changed cells highlighted on the canvas and visible in the three-view preview, undoable — never
  silently at build.

## 9. The generator and the extractor — D-30-9

**The generator** (`tacsim::ground::generate`, `ground-pack gen`) is the shipped authoring source
and grows the passes the spike proved: a base relief (the V3 landscape as preset `proving_ground`,
byte-identical), feature placement (river, ford, roads, compounds, woods boxes — today's constants
as a preset's parameters), the biome vocabulary of §6 (presets `mars_canyon`, `lunar_crater`,
`ice_shelf`, taking the *shapes* PR #173 and PR #178 sketched — the trough and shelves, the two
craters, the ridges — at §5's amplitudes), and the §5 check. Every preset is seeded and
deterministic (`libm`, as V3 is), and a preset that ships is pinned by the content test as the
proving ground is.

**The extractor** moves into the tree as `scripts/ground-extract/` (Python, run with `-I`, reading
`.unitypackage` archives outside the tree — the licensing ruling), emitting a draft with full
`provenance`; its relaxation and speckle passes are *removed* from the script because the writer
runs them. The v4 pack is regenerated through `ground-pack build` from the extractor's draft and
becomes the candidate for **the second shipped arena** (Q-30-7) — the first content that exercises
a biome end to end if it is the Mars mesh, or the first real valley if it is the Winter Pack
terrain.

## 10. Disposition of the spike's PRs

| PR | Keep | Remove or replace | Cut |
|---|---|---|---|
| #154 (merged) | the compositor, the padded-sheet sub-cell trick, the viewport-over-canvas idea | `p=` addressing (w11term `074a20c1` lands the terminal fix), one sheet per model, tiles for the terrain, the doc 21 D-21-10 amendment, a bake script for the mip blob | WG.6 |
| #169 (merged) | `rgb_for_biome`, `zenith_rgb`/`horizon_rgb`, the fog/sky/paint tables, the 4-biome × 3-view test | `Terrain::synthesize_relief`, the `all_flat` branch, `procedural_wanted`, the substring `from_name`; biome read from the wire word only; the `rock`-as-`Wall` legend mapping and the rock-spire drawing (D-30-10) | WG.2 |
| #173 (merged) | the four-word `BIOMES` vocabulary (moves to `RegionDef`), the *shape* of its relief rule (flat musters, scaled roads, bounded, seeded) as generator presets | `ArenaDef.biome`, `ArenaDef.procedural_relief`, `biome_variant`, `apply_biome_and_relief`, the SIN64 relief and the class remap; the 45 cm rule | WG.3 |
| #178 (merged) | `map-editor.ts`'s brush engine, Bresenham, flood fill, undo, the Cells codec, the Svelte shell, the three-view preview | export/import → the draft; `elev_field` → `elev`; the pose keys; the 45 cm clamp → the passability overlay; the height band; the 256 preset; the `@atlas/window` pre-fill; the eight-byte `Cells` codec → the three-byte class runs and `le16` | WG.4 |
| The extractor (outside the tree) | the heightmap and splat reading, the stitching | the pack writing, the relaxation, the speckle handling → `ground-pack` | WG.5 |

## 11. Cuts

| Cut | What | Acceptance | Depends on |
|---|---|---|---|
| **WG.0** | this document; the README row | merged | — |
| **WG.1** | `tacsim::ground`: the draft type and its validation; `generate` moved from `tests/support/ground.rs`; the writer (`build`: quadranting, the two packs, signing); `ground-pack` with `build`, `gen`, `check` (size only); the inverse pack → draft; the proving ground's draft checked in and §4's two-half pin replacing `the_shipped_atlas_pack_is_this_generators_output` | `gen proving_ground` reproduces the checked-in draft and `build` over it reproduces the shipped packs byte for byte; the content pin's digests unchanged | — |
| **WG.2** | picture hygiene: remove `synthesize_relief` and the fallback; exact `Biome` parse; "a flat field draws flat" in `pixel_relief.rs`; `rock` drawn as a surface (D-30-10); palettes untouched | the 4 × 3 distinctness test still passes with `elev` supplied; no mush-pixel path writes an `elev_cm` the feed did not carry; a `rock` cell at `elev_cm` 0 blocks no sight line in any picture | — |
| **WG.3** | the sim side: `RegionDef.biome` + validation; `arena.map.biome` from the region; `ArenaDef.biome`/`procedural_relief` and the load-time passes removed; `check`'s grade and passability report over `slope.rs`/`path.rs`; the repair passes (relaxation, speckle) in `build` with §5's defaults and the built-class exemption; `route` | the proving ground's report: tracked ≥ 90 %, zones mutually reachable, every key spot wheeled-reachable; a hand-made 45 % ridge draft passes `check` and a 50 % one is reported; `route` over the proving ground with its graph emptied reaches every key spot; a one-cell `wall` and a studio draft's 70 % cliff survive `build` | WG.1 |
| **WG.4** | the studio on the draft: open/save `ground.draft.v1`, the server's draft endpoint, the real preview feed, the three-byte class codec and `le16`, the passability overlay, the waypoint layer, the viewshed overlay, the live §5 report, the relax/despeckle tools, 256 cells, the height band, the export route; the fixture test holding the TS report, route and viewshed to `ground-pack`'s | open the shipped proving ground, move a key spot, save, `ground-pack build`, load through `pack_paths`, `RANGE` to the moved spot in a match; the preview at The Bluff matches the first-person frame hash of the same pose on the server; on the fixture drafts the studio's report, route and viewshed equal `ground-pack`'s | WG.1, WG.2, WG.3 |
| **WG.5** | the extractor in the tree; the v4 draft regenerated through `build`; the second shipped arena (Q-30-7) with its pin | the v4 arena boots from the tree's packs; its report meets §5; the pack's `provenance` names the Unity packs and versions | WG.3 |
| **WG.6** | the window on `arena.map` and PR #154's tiles; `p=` addressing on the fixed w11term; one sheet per model; the D-21-10 amendment | a 512 × 512 draft plays over SSH within doc 21 §5's budget; eight tanks in a row all draw | WG.3, w11term `074a20c1` |

WG.1–WG.3 are a week of one builder; WG.2 can run beside WG.1. WG.4 and WG.5 are independent of
each other after WG.3. Each cut records itself in `30-cut-notes.md` in doc 20's manner.

## 12. Amendments owed

- **Doc 16 §3.2** "Terrain": the ground is content (D-30-1); biome as a region property (D-30-5);
  `rock` a surface with no height of its own (D-30-10); §4: the draft and `ground-pack` as the
  authoring route beside the pin. **§11** the window.
- **Doc 21 §7** D-21-14 reaffirmed against the picture (D-30-6); **§8** the heightfield: the one
  encoding, the flat-field rule, the window and the tiles (D-30-7); **D-21-10** amended by WG.6;
  **21a** the inventory row for the 2.5D view.
- **Doc 14** the region record: `biome` beside `body` and `theme`; D-14-28 unchanged.
- **Doc 22** the look half: a biome's palettes are the look's, listed with the sheets.
- **`docs/design/README.md`**: this row.

## 13. Questions for the owner (the lead's defaults until ruled)

- **Q-30-1 — where the writer lives.** Default: `tacsim::ground` with the `ground-pack` binary in
  `crates/tacsim/src/bin/`, because the check needs `slope.rs` and `path.rs`; a `mush-ground` crate
  below tacsim could not call them. Alternative: a crate with the check as a tacsim feature.
- **Q-30-2 — biome and body.** Default: independent fields; a region on `luna` may be temperate (the
  proving ground is, and stays). Alternative: `body` implies a default biome the region may override.
- **Q-30-3 — road grade.** Default: roads authored ≤ 20 % (wheeled 25 % with a margin, the v4
  figure); off-road at each drive's limit.
- **Q-30-4 — passability acceptance.** Default: tracked ≥ 90 %, legged ≥ 95 % of land cells from
  both entry zones; the zones mutually reachable by tracked, wheeled and legged; every key spot
  wheeled-reachable.
- **Q-30-5 — the studio's route to the server.** Default: download → `ground-pack build` →
  `pack_paths` → restart for WG.4; `@ground/stage` (sign and hot-shelve a draft as an operator pack)
  as a later cut once the hot-load half of the shelf exists.
- **Q-30-6 — the biggest draft.** Default: 512 × 512 at 10 m (sixteen 128-cell quadrants, the
  `MAX_ARENA_TILES` cap), which forces WG.6's window. Alternative: hold at 256 until the window
  lands.
- **Q-30-7 — the second shipped arena.** Default: the extractor's Winter Pack valley (a real
  temperate heightfield, no new vocabulary), then the generator's `mars_canyon` as the first
  biome arena. Alternative: Mars first, to exercise the biome end to end sooner.
- **Q-30-8 — the `rock` class.** **Ruled 2026-10-30 (D-30-10, §7), confirmed by the owner the same day**: a surface —
  its cost and shading on the real heightfield, no height of its own; the picture raises only what
  the sim's sight table raises. The former default (a block, as drawn today) and the
  alternative (a `rock`/`cliff` split) are both retired by it.
- **Q-30-9 — the extractor's artifacts as tree content.** Default: yes, per the 2026-10-29 ruling,
  with `provenance` naming the pack and version; the script ships, the `.unitypackage` never does.

## 14. Risks, and what this document refuses

- **Refused:** invention at load or in a picture; a second elevation encoding; biome by name
  sniffing; a clamp that is not the sim's limit; an authoring surface whose export the sim cannot
  load.
- **The pin is the cost of the principle:** every generator change is a reviewed hex diff and a
  regenerated 180 KB file. That is the point (16b C.2), and `ground-pack build` makes the
  regeneration one command.
- **Three sources, one writer:** the studio and the generator drift only if the TS codec drifts
  from the Rust one; the round-trip test (draft → TS encode → Rust decode → compare) in WG.4 is the
  guard, and the same fixture drafts hold the studio's report, route and viewshed to
  `ground-pack`'s (§8): a disagreement is a failing test, never a builder's surprise.
- **The cap:** real relief over a 256-cell grid sits near 256 KiB; WG.3's size check keeps a draft
  from shipping a frame the client refuses, and WG.6 is the real answer. Until then a steep biome
  arena may have to be smaller than 256 cells.
- **The extractor's speckle** is a data-quality problem no filter fully fixes; the report's
  "islands under 3 cells" count is the number to watch, and a hand pass in the studio is the
  remedy the pipeline now allows.
