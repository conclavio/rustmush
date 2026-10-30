# 22. The unit kit — what a new unit defines

**Draft 0.8 — 2026-10-30, U.6 as built.** The bake matrix is in the tree
([22-cut-notes.md](22-cut-notes.md) "U.6"): `mush_pixel::bake` holds the unit pack (format 2,
named entries keyed by look id with provenance), the size and chain picks and the matrix check;
`scripts/unit-bake/bake.sh` is the one command (Unity's batch mode renders, the Rust reducer
reduces); the first bake of all six looks ships (1,043,471 bytes, so the web fetches it, rule 7).
Four things moved from Drafts 0.5–0.7 in the building, each a measurement: rule 7's runs carry the
material in the varint's first byte (a whole material byte measured twice the size); the matrix
check holds a bake's width within half to twice its look's, not to its height's 2 % (barrels and
arms stand past the sim's silhouette); the 2.5D's size pick reads every frame of a chain as one
world span, so the unit's pixels a metre pick the size; and "0.5× halves both" holds only where
the chain has the room — the chain spans 4:1, the baked frames 8:1 (§5). `heavy_mk1` has its own
look (Q-22-5); the scout's `size_cm` is the sim's 520 × 220, not the 580 the old code used.

**Draft 0.7 — 2026-10-30, the questions ruled.** The owner ruled Q-22-1, -2, -4 and -5 at the
lead's defaults the same day (*"rule Q-22-1, -2, -4 and -5 at their defaults"*); with Q-22-3 ruled
in Draft 0.6 the document carries no open question, and U.6 may start.

**Draft 0.6 — 2026-10-30, the review folded.** The author of PR #178 reviewed Draft 0.5 on PR
#184 with four points, all taken (the owner, the same day: *"fold all four points into doc 22 as
Draft 0.6 … with your refinements"*): the `iso` chain reaches 24 px in eight sizes and both
pictures pick a size from the projected height, which also mends two live defects — the 2.5D reads
neither `size_cm` nor its zoom when it picks a mip (§3.3, §5); the `fp` bake pitches 3°, not 8°,
because the heightfield's camera is level, and the heightfield draws from the chain whose pitch is
nearer the line of sight — **Q-22-3 ruled** (§3.3, §5); the hull-down clip is already built, the
heightfield's per-pixel depth test, and becomes a check against the sim's ray (§5, §6); and the
pack gets a size rule with a measured budget, rule 7 of D-22-10 (§3.3). Two looks, `drone` and
`wheeled_scout`, carry no `size_cm` in the tree; U.6 completes them.

**Draft 0.5 — 2026-10-30, the bake matrix (D-22-10) and cut U.6.** The week's pictures — the
heightfield's first person (doc 21 §8), PR #154's fixed-angle 2.5D (merged `4e2f52a9`) and the
Unity extractor's trial — showed that D-22-9's one sheet is not what a unit needs: the two pictures
look at a unit from different pitches, the first-person view needs a size chain down to 8 px, and
the one bake in the tree (`looks/iso25d_mips.bin`) is oblique, positional, two models of five, and
has no script behind it. §3.3 decides the matrix (views × facings × poses × sizes, every size
reduced from the large render, one script, one named asset, provenance in the pack), §5–§6 its
drawing and checks, §8 cut U.6; five questions (Q-22-1..5) take the lead's defaults until the owner
rules. [Doc 30](30-world-generator-v2.md) owns the ground and the biome palettes; this document owns
the unit.

**Draft 0.4 — 2026-10-28, D-22-9 format & 8-way sensor facing landed.** `mush_pixel::look` raises
`SPRITE_MAX` to 96, expands `Facing` to all eight directions (`front`, `front_right`, `right`,
`rear_right`, `rear`, `rear_left`, `left`, `front_left`), accepts all eight facings plus optional
per-facing `wreck` in `Look::parse` while keeping 3/4-facing hand-drawn sheets valid with
nearest-cardinal fallback (`Facing::cardinal`), and `tacsim` adds 8-way `facing` (`ContactFacing`)
on `FrameContact` and `arena.cockpit` contacts alongside 4-way `aspect` (kept for armour facets and
unit minds).

**Draft 0.3 — 2026-10-28, the sheet decision (D-22-9).** A throwaway PoC baked a licensed 3D mech
(the owner's Unity assets) into sheets and into voxel models and drew both in the raycaster
([22-cut-notes.md](22-cut-notes.md) "Sprite vs voxel PoC"): voxels cost ten to twenty times the
draw time for the same bytes on the wire. The owner confirmed units stay **sprites, not models**,
and locked the sheet at **eight facings, six poses, 96 pixels** (D-22-9, superseding D-22-4's
numbers).

**Draft 0.2 — 2026-10-22, U.1–U.3 as built.** The frame names your team and every classified
contact's type; bodies are drawn in team colour; four looks ship with dolls (tank, scout, biped,
drone) and three with sprite sheets (tank, biped, and the `unknown` look a contact not yet
classified is drawn with); a contract test holds every unit type to its look. One change from
Draft 0.1, made building U.3: a sheet's palette names **materials** from a closed set rather than
free RGB colours (§3.2), so every frame stays inside its 256 colours by construction.

**Draft 0.1 — 2026-10-22.** Written from the owner's ask, after the text cockpit's live smoke:
"as we work on the sprite sheets, I'd like to use it as an opportunity to template out what a
'new unit' needs to define … the list of sections, the doll structure … the sprite sheet with a
list of required poses/orientations … list of weapons per section … we have a 'tank' and a 'mech'
type now, but eventually we'll have a dozen+ different units, and probably player-designed units
eventually". The owner accepted every default proposed (D-22-1..8) and ruled the one question
raised after it (D-22-5). Cuts U.0–U.3 build it; [22-cut-notes.md](22-cut-notes.md) records them.

## 1. Two halves

A unit is two things, kept apart on purpose:

- **Its rules** — what it is in the simulation: sections, armour, what losing each costs, what is
  fitted where, how it moves, what it sees. These exist today as tacsim rules packs
  (`crates/tacsim/content/units/*.json`), digested, checked at load, and folded into the match
  hash. §2 documents them as the template's first half; this document changes none of it.
- **Its look** — how it is drawn: its name on a screen, the doll its damage is painted onto, the
  sprite sheet the picture draws it from. None of this exists today: the text doll and the pixel
  doll are hard-coded to the tank's five section names (a biped gets no doll), and every unit is
  drawn as the same side-on tank. §3 is the new half.

**D-22-1** The look is **not** part of the rules pack. Art changes on its own schedule and must
never move a rules digest or the simulator's hash; the renderers that read a look live in the
gateway half (and, later, the web client's WebAssembly), which never loads rules packs. A look
names the unit it draws, and a contract test holds the two halves to each other (§6).

## 2. The rules half (as it stands)

One JSON pack per unit type, `{manifest, entries: {"tacsim.unit.<id>": {kind: "unit", def}}}`,
embedded by `crates/tacsim/src/definitions/content.rs`; the struct is `UnitSpec`
(`definitions/unit.rs`), every level `deny_unknown_fields`. Fourteen types today: the line tank and
its four variants, `heavy_mk1`, `target_mk1`, `scout_mk1` (wheeled), the walkers `biped_mk1` and
`biped_mk2` (the "mech"), `drone_mk1`, the `dray` haulers and `tv_prospector`.

| Field | What it says | Notes |
|---|---|---|
| `label` | the name | "Mk1 biped" |
| `sections[]` | `id`, `facets[] {id, armor}`, `internals`, `consequences[]`, optional `struck_through {section, facet}` | at most 16 sections, 8 facets each |
| `consequences` | closed set: `destroy_unit`, `remove_weapons {section}`, `slow {percent}`, `no_turn_toward {side}`, `disable_motion`, `disable_motion_when_all_of {sections}`, `degrade_core`, `destroy_core` | what losing the section costs |
| `components[]` | `{id, section, role}`, role one of `weapon {weapon, coaxial}`, `sensor`, `ecm`, `eccm`, `power`, `core` | at most 32; one core |
| weapons | a catalogue of their own (`content/weapons/*.json`): delivery, `mount {section, facet}`, bands, cycle, ammunition, burst, muzzle speed, heat | **weapons per section**: a weapon is fitted by a component on a section and mounted on the weapon's own mount section where the unit has it; `remove_weapons` on a section takes every weapon mounted there |
| `mobility` | one of `tracked`, `wheeled`, `legged`, each `max_speed_cms`, `accel_cms2`, `brake_cms2`, `turn_rate_mdeg_s`, `reverse_factor_pct` | |
| `facets[]` | `{band, sections [{section, weight}]}` | which section a hit on a face lands on |
| `silhouettes[]` | `{band, width_cm, height_cm}` | ballistics, not art |
| `sensor_height_cm`, `heat_max`, `heat_dissipation_per_s`, `class`, `logistics`, `interior`, `note` | | |

Not there yet, and not this document's: mass on combat units, crew stations, a power budget.

## 3. The look half (new)

One look a unit type, a JSON file keyed by the unit's definition id:

```json
{
  "unit": "tacsim.unit.line_tank",
  "label": "Line tank",
  "short": "LT",
  "weapons": { "cannon": "105", "coax": "MG" },
  "doll":    { ... },
  "sprites": { ... }
}
```

- `label`, `short`: what a screen prints for the type ("Line tank", "LT"); `weapons`: a short name
  per weapon component id for the text panel's weapon rows (a component it does not name prints
  its id).

### 3.1 The doll — D-22-2

The picture of the unit its damage is painted onto: a grid of cells, at most **24 wide by 16
tall** (smaller allowed — a drone needs few), written as rows of characters with a legend naming
the section, and optionally the facet, each character stands for; `.` is empty.

```json
"doll": {
  "view": "top",
  "rows": [
    "llll.FFFFFFFFFFFFFF.rrrr",
    "llll.HHHHHTTTTHHHHH.rrrr",
    "llll.HHHHTTTTTTHHHH.rrrr",
    "llll.HHHHHTTTTHHHHH.rrrr",
    "llll.HHHHHHEEHHHHHH.rrrr",
    "llll.RRRRRRRRRRRRRR.rrrr"
  ],
  "legend": {
    "F": { "section": "hull", "facet": "front" },
    "R": { "section": "hull", "facet": "rear" },
    "H": { "section": "hull" },
    "T": { "section": "turret" },
    "E": { "section": "engine" },
    "l": { "section": "left_track" },
    "r": { "section": "right_track" }
  }
}
```

A cell is coloured by what is left of its section (or of its facet, where the legend names one):
green, yellow, red, grey when gone — the instruments' ramp. `view` is `top` (a vehicle from above,
front up) or `upright` (a walker standing, drawn with its left on the left, as its pilot thinks of
it). Every renderer draws the same grid: the text panel
in half-blocks, two grid rows to a text row (24x16 is 24 columns by 8 rows), the pixel and web
dolls scaled. Every section the unit has must appear in the grid at least once; every legend
character must be used and name a section the unit has (and a facet that section has).

### 3.2 The sprite sheet — D-22-3, D-22-4

How the picture draws the unit standing in the world. **Pixel art as text** for this first form:
each frame a block of rows, one character a pixel, a palette naming each character's colour.
Readable, diffable, needs no image decoder in `mush-pixel` (which builds for WebAssembly), and
the form a player-designed unit is written in; a PNG importer can produce it later.

```json
"sprites": {
  "size": [32, 16],
  "size_cm": [700, 300],
  "anchor": [16, 15],
  "palette": {
    "a": "team-dark", "b": "team", "c": "team-light",
    "k": "black", "s": "steel", "p": "pale"
  },
  "facings": {
    "front": { "still": [ ["...rows..."] ], "moving": [ [...], [...] ], "muzzles": { "cannon": [16, 4] } },
    "rear":  { ... },
    "right": { ... }
  },
  "wreck": [ ["...rows..."] ]
}
```

- **The required poses**: the facings `front`, `rear` and `right` — `left` is `right` mirrored
  unless the sheet gives it; for each facing one `still` frame and two to four `moving` frames
  (the tread or walk cycle, played while the contact is moving); one `wreck` frame (or one a
  facing); a muzzle point a weapon a facing, where a shot's flash is drawn.
- **Superseded by D-22-9 (2026-10-28): eight facings, six poses, 96 pixels.** Every baked sheet gives
  all eight facings — `front`, `front_right`, `right`, `rear_right`, `rear`, `rear_left`, `left`,
  `front_left` — each its own art, never a mirror (a unit's weapons are not symmetric: a mirrored
  left moves the cannon to the other arm); six poses a facing: `still`, a four-frame `moving` cycle,
  and a `wreck` (either per-facing `facings.<dir>.wreck` or up to eight top-level `wreck` frames);
  frames up to 96 by 96 (`SPRITE_MAX` is 96), drawn for a larger picture than
  today's 640-wide frame and for the web's canvas. Sheets are produced by the bake pipeline
  (`scripts/unit-bake/`) from 3D models; 3/4-facing hand-drawn sheets remain valid input and fall
  back to their nearest cardinal facing (`Facing::cardinal`) when asked for a diagonal. The cockpit
  frame carries an eight-way `facing` (`ContactFacing`) alongside four-way `aspect` (which stays
  four-way for armour facets and unit minds) so `cockpit::contacts_as_sprites` picks the exact
  diagonal when present. The text below is the rule as U.3 built it.
- **Four facings now, room for eight** (D-22-4): the sim tells a viewer only which face a contact
  was showing — `aspect`, front/left/right/rear off the plot, by design — so four is what a sheet
  can be chosen by. The format names facings by word; `front_left` and the other diagonals are
  reserved for the day the owner lets a sensor say more.
- **Materials and team colour** (D-22-3, D-22-7 as built): a palette maps each character to one
  of six materials — `team-dark`, `team`, `team-light`, `black`, `steel`, `pale` — never to a
  colour. The three team slots are recoloured when the sprite is drawn: red for the red team, blue
  for the blue, a neutral grey while the contact is unknown; a wreck has no team (its team slots
  draw black, burnt and steel). The renderer fogs the materials in six steps (the ground has
  twelve), so the ground's bases, the sky and the sprites are exactly 256 colours.
- `size` is the frame in pixels, `size_cm` how big the unit stands in the world (the picture
  scales the frame to it at the contact's range), `anchor` the pixel that stands on the ground.
  A dark ground shadow is drawn under every unit at its anchor, by the renderer, not the sheet.

### 3.3 The bakes — D-22-10 (Draft 0.5)

D-22-9 fixed the sheet. The week after it, the pictures multiplied: the heightfield's first person
(doc 21 §8) looks at a unit from a 1.5 m eye, PR #154's 2.5D from a fixed 28° above, and both
draw it from a few pixels to 96. One sheet at one pitch is not what a unit needs; a unit needs a
**bake matrix** — views × facings × poses × sizes — made by one script from one named asset. The
tree does not have it yet.

**As it stands (2026-10-30).**

- The raycaster (flat ground) draws a contact from its look's text sheet (U.5's art: the tank
  56x20, the biped 24x50, `unknown` 32x16), nearest-sampled to `size_cm` at range.
- The heightfield picture (G.7, the default over relief) and the 2.5D both draw from
  `looks/iso25d_mips.bin` (PR #154): per model six mips of 96, 86, 78, 70, 64 and 56 px
  (`MIP_SIZES`), each eight facings by six poses, baked at the 2.5D's pitch. The pack is positional
  (`M25D`, version 1, a count): two models — the biped from the Fox light mech, the tracked tank;
  `unknown` is re-rendered from its text sheet (`fallback_mip_from_sheet`); `wheeled_scout` and
  `heavy_mk1` are `synthesize_3d_mip`, a procedural placeholder; the drone has no slot. No script
  in the tree produced the pack.
- So the first-person view draws an oblique bake at a near-level eye (top surfaces showing,
  silhouettes wrong), stops at 56 px and nearest-samples below it — a tank at 300 m in the 640
  frame is 11 x 4 px, U.5's whole lesson — and two of the shipped looks are synthesised. The
  owner's observation of 2026-10-29: the mips are less crisp than U.5's render-large-then-reduce
  sheets, because they were baked at target size.
- The heightfield's camera is level — its horizon is the frame's middle row; there is no
  down-look — so the only pitch to a contact is its line's own elevation angle, near 0° on level
  ground at any range; an 8° bake shows a roof on every contact (Draft 0.6, the review).
- The 2.5D picks a mip from the contact's screen row and the viewport width alone
  (`pick_mip_level`) and blits it at its native size: `size_cm` and the camera's zoom
  (`set_zoom_permille`, 0.5×–2×) play no part, so a drone draws at a heavy tank's size and at
  0.5× the ground halves while every unit keeps its pixels. The six sizes' ~10 % steps were a
  depth cue for a parallel projection, not a scale (Draft 0.6, the review).
- The heightfield already clips a contact against the ground: its march writes a per-pixel depth,
  trees and structures write into it, and the sprite blit skips every pixel whose terrain is
  nearer (`draw_mip_sprite_fp`), so a tank behind a crest shows only the rows above it. Nothing
  holds that to the sim's ray yet (Draft 0.6, the review).
- `scripts/unit-bake/`: `Bake.cs` renders orthographic at 384 px at one pitch (`Elev` 8°), eight
  facings, six poses, a colour-ID pass and a normal pass, anchors and muzzles in `meta.json`, and
  the PoC's voxel grids; `reduce.py` reduces to the six materials at 48 and 96.

**D-22-10 — the bake matrix.** For every shipped look, one bake produces:

| Axis | Values | Why |
|---|---|---|
| View | `fp` — a 1.5 m eye, pitch **3°** (Q-22-3, ruled in Draft 0.6; `Bake.cs` has 8° today); `iso` — the 2.5D's fixed 28°; `top` reserved (Q-22-2) | the two pictures look from different heights; one bake cannot serve both; a contact draws from the chain whose pitch is nearer its line of sight (§5) |
| Facing | eight at 45°, each its own art (D-22-9); Q-22-1 on sixteen for `iso` | |
| Pose | `still`, `moving` × 4, `wreck` (D-22-9) | |
| Size | `fp`: 96, 64, 48, 32, 24, 16, 12, 8 px; `iso`: 96, 80, 64, 56, 48, 40, 32, 24 px (Draft 0.6: eight sizes, every step under 25 %; the 2.5D's 96, 86, 78, 70, 64, 56 retired with its pick) | a contact draws from the size nearest above its projected height — from `size_cm` at its range in the heightfield, through the projection and the zoom in the 2.5D (§5) — never resampled from a smaller one; `fp`'s chain reaches 1 km in the 640 frame (Q-22-4 on the floor); `iso`'s reaches a drone at 0.5× |
| Material | the six materials through the colour-ID pass, team slots recoloured at draw (D-22-3, D-22-7); at most 16 colours a sheet | the frame's 256 colours by construction |
| Marks | anchor, height (the 2.5D's depth sort, the lock box) and a muzzle a weapon — per view, per facing, per size | the pitch moves them |

The rules:

1. **Every size is reduced from the large render** — the 384 px pass or larger — never from a
   smaller size. Masses, not lines (U.5); the reducer is the only resampler, and the owner's
   crispness observation is this rule's reason.
2. **One script, one named asset.** `Bake.cs` grows the per-view pitch and emits both views in one
   run; the reducer (`reduce.py`, or a Rust reducer under `mush-pixel`'s examples — the builder's
   call) emits the chains. The pack records its provenance — the asset pack's name and version,
   the prefab and clip names, the bake and reduce commits — which is the 2026-10-29 licensing
   ruling's corollary; the assets and the raw renders never ship (`scripts/unit-bake/README.md`).
3. **The look JSON stays the player's source** (D-22-7): one hand-drawable 96 px text sheet,
   `size_cm`, anchor, muzzles. The baked chains are a binary pack generated from the bake, keyed
   by **look id** — named entries and a version, replacing `M25D`'s positional header — and a
   chain must agree with its look's `size_cm` and anchor (the `drone` and `wheeled_scout` looks
   carry no `size_cm` in the tree today; U.6 completes them). A look with no chain draws its text
   sheet nearest-sampled in every picture, as today: the path a player-designed unit takes.
4. **The roster rule.** Every look under `looks/` (the biped, the tracked tank, the wheeled scout,
   the drone, `unknown`) has both chains at every size; `synthesize_3d_mip` goes, and the
   sheet-to-chain fallback stays for unshipped looks only. `heavy_mk1` has a 2.5D slot and no look
   (Q-22-5).
5. **Which picture reads which chain** (§5): the raycaster the `fp` chain; the heightfield the
   chain whose pitch is nearer the contact's line of sight — `fp` within about 15° of depression,
   `iso` beyond it (Draft 0.6); the 2.5D the `iso` chain; the web's canvas the `fp` chain at its
   own scale.
6. The voxel grids leave the bake: D-22-9 chose sprites.
7. **The pack's size** (Draft 0.6, the review's fourth point). A frame is stored as its tight
   bounding box, run-length coded with a material byte and a varint run — `M25D`'s runs are one
   byte, three material bits and a five-bit length capped at 32, and the transparent field around
   a sprite is what a chain pays for. The bake prints the pack's size and the matrix check (§6)
   pins it against a budget: **512 KiB embedded**. The figures: today's pack is 410 KB for two
   models at one view, eight times under its raw bytes; the full matrix — six looks, both chains —
   is about 13 MB raw and about 1.6 MB at today's packing, so the budget is a measurement U.6
   makes, not a promise. Past it, the native binaries keep embedding the pack and the web fetches
   it as a separate cached asset instead of carrying it inside `mush-pixel-wasm`, which drops
   flate2 and so has no free inflate.

**Questions — all ruled 2026-10-30** (Q-22-3 in Draft 0.6, the rest in Draft 0.7, the owner at the lead's defaults).

- **Q-22-1** Sixteen facings for `iso`? **Ruled 2026-10-30 at the default:** eight: the 2.5D turns a unit in 45° steps today,
  and sixteen doubles every `iso` sheet.
- **Q-22-2** A `top` view for the overhead? **Ruled 2026-10-30 at the default:** no: the overhead keeps its scene markers (doc
  21 §7.4); an icon is a later cut if the overhead ever zooms past a few pixels a unit.
- **Q-22-3** The `fp` pitch. **Ruled 2026-10-30 (Draft 0.6): 3°.** The heightfield's camera is
  level, so the 8° default showed a roof on every contact on level ground; 0° would lose the roof
  and with it the body's solidity; 3° keeps a sliver. `Bake.cs`'s `Elev` moves with it.
- **Q-22-4** The `fp` floor. **Ruled 2026-10-30 at the default:** 8 px; under it the bracket alone, which is the bracket rule.
- **Q-22-5** `heavy_mk1`. **Ruled 2026-10-30 at the default:** a look and a bake with the rest, so the 2.5D slot stops being
  a synthesised shape.

**As built (U.6, Draft 0.8).**

- **The pack** (`crates/mush-pixel/looks/units.bin`, `mush_pixel::bake`): magic `MUPK`, version
  2; per entry the look id (its file's stem), the provenance (asset pack and version — the package
  file's SHA-256 prefix when the config names none —, the assembled prefab and its parts, the
  clips, the bake and reduce commits), `size_cm`, the muzzle names, and per view its span (the
  world a frame's side covers: one scale for the whole chain) and the chain; per frame the tight
  box, anchor, height, muzzles and runs. Rule 7's run is the material in the low three bits of a
  varint's first byte with four bits of length, so a run of up to sixteen is one byte: measured on
  the shipped M25D art, the literal material byte plus varint came to 3.1 MB for six looks against
  1.6 MB. The first bake is 1,043,471 bytes for six looks, the `iso` chain about twice the `fp`:
  over the 512 KiB budget, so the native binaries embed it and the web's WebAssembly build embeds
  none — the page fetches `units.bin` beside the module and hands it to `mp_units`.
- **The bake** (`scripts/unit-bake/`): one config a look (`looks/<look>.json`: the package, the
  prefab and the parts in their mounts, the colour-ID texture or a material-to-part map, the clips,
  the muzzle locators by weapon id, `size_cm`); `Bake.cs` scales the model to its look's height,
  frames each view on the unit's turning circle, holds a moving frame's middle over the still's (a
  drive clip's root motion drove the tank out of its frame in the first bake), tilts a wreck with
  no fall clip; `bake.sh` imports, bakes, reduces and prints the pack's size, digest and the matrix
  check. The reducer is Rust (`bake::reduce`, `examples/unit_reduce.rs`), tested on a synthetic 384
  px render; `reduce.py` and the voxel grids are gone.
- **The width rule.** The matrix check holds a bake's height to its look's within 2 % (the bake
  scales to it) and its widest silhouette within half to twice its look's: the PoC walker measures
  9.6 m across its cannon against its look's 5.2 m. The kit's vehicles are squat — the tank 4.1 m
  long at its look's 2.5 m height, the sim's 7 m — and draw their bake's width, not the look's: a
  longer model is a config change.

## 4. Teams and colours — D-22-3, D-22-5

In the arena, **bodies are drawn in their team's colour** — red-team red, blue-team blue,
whoever is looking — so a pilot on the red team is never told "red means enemy". Unknown is a
neutral grey; a wreck is burnt brown. Friend or foe is the markers' job (the lock box, the
contacts list, the tape's pips), never the body's.

The cockpit frame gains what this needs:

- the pilot's own `team` and own `unit_type` (the definition id), top level;
- per contact, `unit_type`, **from classification** — the owner's ruling of 2026-10-22 (D-22-5),
  taken after being shown that the sim had held a contact's chassis back until a close reading
  (16b cut Q.2: "a section id is a chassis tell"). The type is now known with the team; the
  section **condition** stays behind a close reading as before. A contact not yet classified has
  no type and draws as the generic `unknown` look.

Neither is a frame version: fields added for the person in the seat are not one (doc 17's rule).

## 5. Drawing

- **Text panel** (the TUI's text cockpit, doc 21 D-21-10 revised): the armour column draws the
  pilot's own unit's doll from its look in half-blocks, then a line a section.
- **Pixel picture**: each contact is drawn from its type's sheet — facing from `aspect`, `moving`
  frames cycled on the picture's clock while the contact is moving (a step every three draws, 150
  ms at 20 Hz), the team slots recoloured — scaled to `size_cm` at its range, standing on the
  ground at its anchor with a shadow under it; the bracket rule stands (a contact the sim reports
  is never invisible). An unknown contact uses the `unknown` look (the reserved unit id
  `unknown`); a type whose look has no sheet is the old silhouette, in team colour. Fog dims a
  sprite as it dims the ground.
- **Web**: the cockpit tab's doll and the picture's sprites read the same looks (a later cut).
- **As built (PR #154), and under D-22-10**: the heightfield picture and the 2.5D draw a contact
  from the baked chains — today from `looks/iso25d_mips.bin`, the one oblique bake, at both
  pitches, which is the defect U.6 fixes; the raycaster from the look's text sheet. A look with no
  chain draws its sheet nearest-sampled in every picture.
- **The size pick, both pictures** (Draft 0.6): a contact draws from the size nearest above its
  projected height — the heightfield's from `size_cm` at its range, the 2.5D's from `size_cm`
  through the projection and the zoom, the row's depth cue a factor on that height rather than a
  fixed step — so a drone and a heavy on one row stand in the ratio of their heights, and 0.5×
  zoom halves a unit as it halves the ground. *As built (U.6):* every frame of a chain spans one
  world span, so the pick is the smallest size at or above the frame side the unit's pixels a
  metre ask for (`Chain::pick`); the heightfield scales that size down to the contact, the 2.5D
  blits it natively at nine pixels a metre at 1.0× on a 640 px viewport, the top row at 0.85 of the
  bottom's. The chain spans 4:1 and the baked frames 8:1 (the drone's 2.2 m to the walker's
  17.8 m), so no one scale keeps every unit inside the chain at every zoom: the drone draws on the
  24 px floor within a chain step of its ratio to the heavy, the tank, the heavy and the scout
  halve at 0.5× (to the chain's step: 56 has no half), the walker stands on the 96 px ceiling and
  steps down to 80.
- **The chain pick** (Draft 0.6): the heightfield draws a contact from the chain whose pitch is
  nearer the line to it — `fp` (3°) within about 15° of depression, `iso` (28°) beyond it — so
  looking down into a bowl from a hill shows the view the 2.5D bakes. The marks come with the
  chain.
- **Hull-down, as built**: the heightfield's ground march writes a per-pixel depth, trees and
  structures write into it, and the sprite blit skips every pixel whose terrain is nearer, so a
  contact behind a crest shows only the rows above it — the picture's half of doc 30 §5's
  hull-down ground. §6 holds it to the sim's ray.

## 6. Checks — D-22-6

- `mush-pixel` checks a look on its own: grid and frame bounds, legend and palette closure, the
  required facings and frame counts, at most 16 colours, frames all of `size`.
- A contract test (`mush-contract-tests`, which sees both halves) holds every shipped unit type to
  a look and every look to its unit: the unit exists, every section is in the doll, every legend
  section and facet exists on the unit, every `weapons` key is a weapon component of the unit,
  every muzzle names one.
- A unit type with no look draws as the `unknown` look and its pilot gets the section lines with
  no doll — never a failure.
- **The matrix check** (D-22-10, U.6): the baked pack's header names each look and its
  provenance; every shipped look has both chains at every size; every frame's anchor, height and
  muzzles lie inside it; a chain's `size_cm` and anchor agree with its look's; the pack's digest
  is pinned, and regenerated on purpose; the pack's size is under rule 7's budget, or the web's
  fetch path is on.
- **The hull-down check** (Draft 0.6): on a fixture of lips — a 2 m rise at 100 m, 200 m and
  500 m before a 2.5 m-tall contact — the sim's ray to the contact (`horizon.rs`) and the
  heightfield picture agree: seen draws at least one row of the contact above the crest, unseen
  draws none. The 2.5D's check: a drone and a heavy on one row draw in the ratio of their
  heights, and 0.5× zoom halves a unit as it halves the ground.

## 7. Player-designed units — D-22-7

The path is the two checks above plus the rules half's own (`UnitSpec::check` and the catalogue
build's cross-checks): a designed unit is a rules pack and a look, each refused with a reason when
it does not hold. A builder, its economy and who may field what are their own design; this
document only makes sure the template is complete and checkable, and that its art is text a
player can write.

## 8. Cuts

| Cut | What | Done when |
|---|---|---|
| **U.0** | this document, its cut notes, the README row | — |
| **U.1** | the frame's own `team` and `unit_type` and each classified contact's `unit_type`; bodies in team colour in both producers; the friendly tint retired from bodies | a red pilot sees red teammates red and blue enemies blue; an unknown contact grey |
| **U.2** | the look format in `mush-pixel` with its check; looks for the line tank family and the biped; the text and pixel dolls drawn from the grid; the contract test | a biped pilot gets a front-view doll; every shipped unit has a look or is listed as drawing `unknown` |
| **U.3** | the sprite sheets: facings from `aspect`, the moving cycle, team slots, scale from `size_cm`, the shadow; placeholder art for the tank and the biped | a walker and a tank read differently at 300 m, turn their faces as they turn, and roll or walk while moving |
| U.4 | the web client's doll and sprites from the same looks | — |
| **U.5** | the art: the tank, the biped and `unknown` redrawn for what the picture makes of a sheet at 100 m, 300 m and 1 km; the shipped sheets held to standing on their anchor, muzzles on the weapon, a cycle that moves, a wreck that is not the still; `examples/sheets.rs` | a tank reads as a tank and a biped as a biped at 300 m; the PNG importer deferred |
| **U.6** (built 2026-10-30, [cut notes](22-cut-notes.md)) | the bake matrix (D-22-10): `Bake.cs` with both views (`fp` at 3°), the reducer emitting the `fp` and `iso` chains from the large render, the named-entry pack format with provenance and rule 7's tight-box RLE, `synthesize_3d_mip` deleted, every shipped look baked (`heavy_mk1` per Q-22-5), `size_cm` on the `drone` and `wheeled_scout` looks, the 2.5D's projected-height pick (`pick_mip_level` goes) and the heightfield's chain-by-elevation pick, the look check extended to the matrix and the hull-down fixture, the pack's digest and size pinned; the line-ups at 100 m, 300 m and 1 km under [images/22-u6/](images/22-u6/) as U.5 did | both pictures draw every shipped unit from its own view's chain at every range; a tank at 300 m in the first-person frame reads as a tank from the 12 px size, not from a resampled 56; a drone and a heavy on one 2.5D row stand in the ratio of their heights and 0.5× halves both; a contact behind a crest shows rows above it exactly when the sim's ray sees it; the pack under 512 KiB embedded, or fetched by the web; no synthesised sprite in a shipped build |

## 9. Decisions

- **D-22-1** The look is a separate file keyed by the unit's definition id, never in the rules pack
  or the hash.
- **D-22-2** The doll is a grid at most 24x16, rows of characters and a legend of section and
  facet, `top` or `upright` view; every section appears.
- **D-22-3** Bodies in team colour (red, blue; unknown grey; wreck brown) through three team
  palette slots; friend or foe is the markers'.
- **D-22-4** Facings front, rear, right (left mirrored), two to four moving frames, a wreck frame,
  muzzle points; four facings now from `aspect`, eight reserved.
- **D-22-5** A contact's unit type is known from classification, with its team (the owner,
  2026-10-22, superseding 16b Q.2's chassis-tell rule for the type; condition still waits on a
  close reading).
- **D-22-6** A look is checked on its own and against its unit by a contract test; a unit with no
  look draws `unknown`.
- **D-22-7** Sprites are pixel art written as text; a palette names materials from a closed set
  of six (as built in U.3: Draft 0.1 had free colours, at most 16).
- **D-22-8** The cuts U.0–U.5 above.
- **D-22-9** (the owner, 2026-10-28, after the sprite-vs-voxel PoC) Units are drawn as sprite
  sheets, not 3D or voxel models. A sheet has **eight facings, each its own art (no mirrored
  left), six poses a facing (still, four moving, wreck), 96x96 frames** — chosen for where the
  picture is going (a larger native frame once optimised; the web's wasm renderer at the client's
  resolution), not today's 640-wide frame, where 96 beats 48 only inside about 25 m. Supersedes
  D-22-4's facing and frame counts; muzzle points stay. The owner confirmed the same day that it
  carries the sensor ruling D-22-4 reserved: a contact's aspect reaches the viewer in eight
  directions, not four (the armour facets the sim hits stay four).
- **D-22-10** (Draft 0.5, 2026-10-30, after the week's pictures) A shipped unit's art is a **bake
  matrix** — two views (`fp`, `iso`) × eight facings × six poses × a size chain each — produced by
  one script from one named asset, every size reduced from the large render, keyed by look id in a
  named-entry pack with provenance; the look's text sheet stays the player's source and the
  fallback. Cut U.6 builds it; Q-22-1..5 (§3.3) are all ruled, the owner on 2026-10-30.
  Draft 0.6 (2026-10-30, the PR #184 review folded): the `iso` chain reaches 24 px and both
  pictures pick a size from the projected height; the `fp` bake pitches 3° and the heightfield
  picks the chain nearer the line of sight (Q-22-3 ruled); the hull-down clip, already built, is
  held to the sim's ray; the pack has a size rule and a measured budget (rule 7).
