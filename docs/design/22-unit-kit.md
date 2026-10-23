# 22. The unit kit — what a new unit defines

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

## 6. Checks — D-22-6

- `mush-pixel` checks a look on its own: grid and frame bounds, legend and palette closure, the
  required facings and frame counts, at most 16 colours, frames all of `size`.
- A contract test (`mush-contract-tests`, which sees both halves) holds every shipped unit type to
  a look and every look to its unit: the unit exists, every section is in the doll, every legend
  section and facet exists on the unit, every `weapons` key is a weapon component of the unit,
  every muzzle names one.
- A unit type with no look draws as the `unknown` look and its pilot gets the section lines with
  no doll — never a failure.

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
| U.5 | real art, a PNG importer to the text form | — |

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
