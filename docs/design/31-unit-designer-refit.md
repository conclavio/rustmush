# 31. The unit designer and the refit bay

**Draft 0.1 — 2026-10-30, docs only.** What comes after doc 22. The owner asked for a unit creator
and editor that also does repairs, "like the map editor", with every Mech Constructor assembly
pre-baked and pushed to a client only when a unit drawn from it is on that client's map. The owner
ruled the four framing questions the same day (R1–R4, §1); the lead's five design points are
carried as decisions at their defaults (D-31-1..14). Two questions stay open (§10). Nothing here is
built; cut UD.0 is this document.

It stands on doc 22 Draft 0.8 and its cut U.6 (PR #188, open on
`claude/mystifying-galileo-4b3706` as this is written): the unit pack format 2
(`mush_pixel::bake`, magic `MUPK`, named entries keyed by look id with provenance), the one-command
bake (`scripts/unit-bake/bake.sh`: Unity batch mode renders, the Rust reducer reduces) and the first
bake of six looks, 1,043,471 bytes. Where this document says "as built" about the pack, it means
that branch.

## 1. The owner's rulings (2026-10-30)

- **R1 — the model/look split: yes.** (*"sounds good to me"*.) A **model** is one baked assembly
  — the art; a **look** is how one unit type is presented — its label, short name, weapon names,
  doll, `size_cm`, anchor, and the model it is drawn from. Models live in a catalogue keyed by
  **model id**; a look names its model. This amends D-22-10, which keyed the baked chains by look
  id (D-31-1).
- **R2 — sprites stay whole under damage, for now.** Damage shows on the doll (doc 22 §3.1) and,
  at the end, in the wreck pose; a unit that has lost its arm still draws with its arm. The owner:
  *"kitty accepts patches against images, so there's a line of sight to doing it reasonably
  cleanly, but not required for where we are at right now."* The later route is recorded in §8
  (D-31-13).
- **R3 — designers are staff only, for now.** Player-designed units stay the long-run path of
  D-22-7; nothing here closes it, and §9 lists what a player designer would add.
- **R4 — repair is for IC hulls only.** The owner: *"simulator/arena pods represent virtual units
  and thus there is no repair concept, a repair simulator for training might be something
  interesting later, but not as an initial requirement."* Arena damage is match-scoped and resets
  with the match, as today; the only damage that outlives a match is an IC hull's (docs 23, 27).

## 2. What is in the tree

**The rules half.** One JSON pack a unit type under `crates/tacsim/content/units/` (seventeen
files), `include_str!`ed by `definitions/content.rs`, digest-pinned in `tests/content.rs`, decoded
into `UnitSpec` (`definitions/unit.rs`, every level `deny_unknown_fields`) and refused by
`UnitSpec::check` with a reason. Operators have a second route already: `[tacsim].pack_paths`,
read by the host, verified by the one loader (`pack::load_pack`), ledgered as
`tacsim:pack:{id}@{version}` rows in Redis whose digest the boot gate compares, so a published
version is never re-signed in place (`record.rs`, `store.rs`, `@tacsim/pack`). Both routes land in
one `Catalog::build_fielded`, which the composition root **freezes** at boot. The arena's hash folds
the catalogue's definition count and its ordered definition digests (`arena/hash.rs`, item 3c), and
a persisted unit names its exact `DefinitionRef` digest and fails boot if that definition is gone.

**The look half.** One look a unit type under `crates/mush-pixel/looks/` (six files), checked by
`mush-pixel` alone and against its unit by the contract test (doc 22 §6). Under U.6 the baked
chains are one file, `looks/units.bin`: native binaries (the gateway, so SSH and kitty) embed it;
the web's WebAssembly build embeds none and fetches the whole file beside the module
(`web/scripts/build-wasm.mjs`, `mp_units`), because it is over rule 7's 512 KiB budget.

**The bake.** `scripts/unit-bake/looks/<look>.json` names a package, a base prefab and the parts in
their mounts (`attach`), the clips, the muzzle locators and `size_cm`. `Bake.cs` assembles and
renders; the reducer writes the entry. The six configs are a first pick from the owner's *Mech
Constructor* packs (*Light and Medium Robots*, *Spiders and Tanks*) and a Unity primitive. Measured
in U.6: **78–217 KB a look packed, both chains**, the `iso` chain about twice the `fp`. The assets,
the Unity project and the raw renders never ship (the licensing ruling of 2026-10-29); the pack
records the provenance that lets a licence holder bake it again.

**Refit and repair.** Three layers exist and none is wired to a player or staff verb:

- `mush_tactics::unit::replacement::replace_component` — a pure replacement between two exact
  **registered** unit shapes: the target root may change the unit's rules, but only from its exact
  registered preimage; the journal keeps the displaced component's state for custody. It grants no
  authority and writes no World unit. `definitions/core_replacement.rs` is its one finite adapter
  today (the governed and ungoverned core families, doc 17h M8b).
- `mush_tactics::field_ops::workshop` — doc 27's **H3**, built: `WorkshopBook` with
  `strip_component`, `scrap_hull`, `repair_section`, `reconstruct_section`, `repair_component`,
  `replace_component_in_slot` (a part whose definition **matches the slot's**, from a stripped
  record or new), `rearm_component`, `quote_full_restoration`, a `WorkshopFacilityTariff` with
  provisional costs and IC durations, and work-order records with before/after operational states.
  `tacsim::field_ops` re-exports it and exercises it in tests only.
- Doc 08's industry driver owns recipe validation, facility capability, duration, output
  construction and repair/refit (doc 08 §10, "Work orders, production, and services"), with `industry.core.refit` and
  `unit.maintain_refit` capabilities on a factory or shop scope (`composition/mod.rs`) and the
  `RefitKind` recipes of `hardware/recipes.rs`.

**The map editor** (`web/src/lib/map-editor.ts`, `web/src/components/MapEditorStudio.svelte`) is
the model the owner pointed at: a DOM-free engine, a Svelte studio over it, `ArenaDef` JSON import
and export, and a live preview through `mush-pixel-wasm`. It exports a file a person commits; it
has no in-instance route. Doc 30 makes the ground a signed pack with one writer (D-30-1), and this
document takes the same line for units.

## 3. Models — R1

**D-31-1 The model catalogue.** A model is a baked assembly with an id (`mc.lt.fox_cannon_mg`,
`mc.st.tank_105`, …), its provenance and both chains; the bake config moves from
`scripts/unit-bake/looks/<look>.json` to `scripts/unit-bake/models/<model>.json` with the same keys,
`look` renamed `model`. A look gains `"model": "<model id>"` (absent: the look draws its text
sheet, the D-22-10 rule 3 fallback, which stays the path for a hand-drawn unit). Many looks may name
one model — the line tank's variants already share one look and would share one model under any
look split. `size_cm` and the anchor stay on the look, and the matrix check holds a look to its
model's measured height and width (doc 22 §6, the width rule) instead of a chain to its look.

**D-31-2 Per-model files.** The single `units.bin` becomes one file a model,
`crates/mush-pixel/models/<model id>.<digest prefix>.mupk`: the format-2 entry unchanged in body,
keyed by model id, its name carrying the first twelve hex digits of its SHA-256 so a client can
cache it forever and a re-bake is a new name. A `models/index.json` lists id, digest, size, height
and the bake's provenance line. The format version moves to 3 for the key change; the pack's codec
and rule 7's runs do not. The matrix check runs a model at a time.

**D-31-3 Native binaries embed every shipped model.** The gateway draws SSH and kitty frames
server-side and must never wait on a file: it `include_bytes!`es every shipped model (a build
script reads the index), so the six of U.6 cost it the same million bytes as today. The budget that
matters is disk and the web's bandwidth (D-31-11), not the binary.

**D-31-4 The catalogue is curated and pre-baked.** A sprite is a whole-unit render; parts cannot be
composited at draw (D-22-9), so a design can only choose among assemblies that were baked whole.
The catalogue is therefore the owner's pick of whole assemblies from the *Mech Constructor* packs
and the others the owner holds — each a base, its parts in their mounts, its clips — baked on the
owner's Windows box with `bake.sh`, committed as files under `models/`, never generated in an
instance. A design that wants an assembly not in the catalogue is a request for a bake. The first
catalogue is the six U.6 assemblies renamed; growing it is content work, one config and one command
a model, at the U.6 figure of 78–217 KB a model. A **bake queue** (a staff request that lands as a
model config in a PR, run on the owner's box by hand or by a self-hosted Windows runner) is later
(§9).

**D-31-5 A model knows its mounts.** The model index carries, from the bake config, the parts in
their mounts and the muzzle locators by weapon id. The designer uses it to offer only designs that
make sense for the art: a design's weapon component must be one of the model's muzzle weapons, or
the look must name its weapon `drawn: false` (a hull-internal launcher that has no barrel to see).
This is a look-against-model check, never a rule the simulation reads.

## 4. Designs — the rules data

**D-31-6 A design is a rules pack and a look, nothing else.** A design is exactly what a shipped
unit is (doc 22 §1): a `UnitSpec` in a one-entry rules pack and a look that names a model. The
designer composes a design from closed parts — sections and facets from a chassis template, weapons
from the weapon catalogue (`content/weapons/`), sensors, cores, mobility within the class's bounds
— and never edits raw JSON in front of the user; the JSON is its output. A design is versioned
(`id`, `version`) the way every pack is, and a version, once published, never changes (§2's gate).

**D-31-7 One validator, run in the editor.** Every check a design meets on the server it meets in
the editor first: `UnitSpec::check` and the catalogue build's cross-checks (the design's weapons,
sensors and cores resolve), the look check (`mush-pixel`), the look-against-unit contract test's
rules (every section in the doll, every weapon named, every muzzle a weapon), and the
look-against-model check (D-31-5, the matrix check's height and width rule). They compile into
`mush-pixel-wasm` behind a `design` feature — tacsim's definitions are already pure data and
`serde_json` — so the studio refuses a design with the server's own reason before export. The
server runs the same code at publish; nothing is checked twice by two implementations. (If the
tacsim definitions crate is too heavy for the module, the check becomes its own small wasm module;
the builder measures.)

**D-31-8 Two exits: the repo and the instance.** Like the map editor, the studio exports files: a
rules pack for `crates/tacsim/content/units/` and a look for `crates/mush-pixel/looks/`, downloaded
or (when the studio runs against a dev checkout) written beside it, and a person commits them; a
committed design is shipped content, embedded and digest-pinned like the rest. Unlike the map
editor, the studio can also **publish** a design to the running instance (D-31-9), so staff can
field and play a design the day it is drawn and commit it once it has earned its place. The
committed form and the published form are the same bytes: committing a published design is
copying two files, and the shipped pack then dedupes against the ledger row by digest exactly as an
operator pack on `pack_paths` does today (`Catalog::build_from`).

**D-31-9 The runtime design registry rides the pack ledger.** A published design is an operator
pack whose bytes live in Redis instead of on a path: `tacsim:design:{id}@{version}` holds the pack
and the look, and the existing `tacsim:pack:{id}@{version}` ledger row holds its digest, so the
boot gate's never-re-signed rule, `@tacsim/pack list` and `@tacsim/pack verify` cover designs
without a second ledger. Design ids live in their own namespace, `tacsim.unit.design.<slug>`, so a
published design can never shadow a shipped unit; promoting one into the repo keeps its id. A
design is withdrawn by marking it unfieldable, never by deleting its version: a hull built on it
names its digest forever (§2), and so does every match record that fielded it.

**D-31-10 The match hash folds what it already folds.** The catalogue the arena hashes is the
fielded catalogue, so a published design changes the catalogue's count and digests and therefore
the hash of every match after it — which is correct, since the hash is a statement about the rules
a match ran under. What follows is a recording duty: a match record names the **catalogue
generation** it ran under (the ordered digests, or one digest over them), and a replay rebuilds
that catalogue from the shipped content plus the ledgered designs of that generation. A design's
versions never leave Redis while a record names them (backup and restore of the instance are
doc 16c T-112's; designs are now part of what it must keep).

When a published design becomes fieldable is Q-31-1 (§10).

## 5. Delivery — the overhead is disk and editor bandwidth

The owner's bar: *the only overhead should be disk space and editor bandwidth*. A player's client
pays for the models it draws and nothing else.

**D-31-11 The web fetches a model when it first needs it.** The page no longer fetches
`units.bin`. Each frame's contacts carry a `unit_type`; the client maps type → look → model id →
file name (the look table and the model index ship with the module, a few kilobytes) and fetches
`/units/<model id>.<digest>.mupk` the first time it needs that model, handing it to `mp_units` to
install beside the others. Two triggers: at muster, the match's roster (every type in the match,
known before the first frame) is prefetched, so a fight normally draws baked from its first frame;
and lazily, any type that first appears in a frame (a late join, a spectator, a type introduced
mid-match) is fetched then. Until a model arrives the contact draws its look's text sheet — the
fallback every picture already has — so a slow fetch costs crispness for a second, never a blank.
The files are immutable and content-named, so they carry a year's `Cache-Control` and a client
fetches each model once per bake, ever. A design published in the instance has no new art (its
model is shipped, D-31-4), so publishing never sends a model to anyone; it sends a look, a few
hundred bytes, in the roster.

**D-31-12 The editor pays for the catalogue, on demand.** The studio loads the model index and
fetches a model only when staff pick it for preview; browsing the catalogue's list costs the index.
That is the "editor bandwidth" of the bar, and it is a staff cost.

Rule 7's 512 KiB budget stops meaning "the whole pack" and becomes a **per-model** ceiling for the
web (the largest U.6 model is 217 KB); the matrix check pins it a model at a time.

## 6. The designer — staff only (R3)

A studio beside the map editor, the same shape: a DOM-free engine (`web/src/lib/unit-designer.ts`)
and a Svelte studio (`UnitDesignerStudio.svelte`), a tab of the builder panel the map studio
sits in (`WorldgenPanel.svelte`), shown to staff only. One screen, four panes:

1. **Chassis and model.** Pick a chassis template (the shipped units' section layouts: tracked,
   wheeled, legged biped, drone) and a model from the catalogue that fits it (the index's class and
   mounts, D-31-5). The preview draws the model's `fp` and `iso` chains through `mush-pixel-wasm`,
   all eight facings and the six poses, at the sizes the pictures pick — the same draw the cockpit
   does, as the map studio previews the ground.
2. **Rules.** Sections and facets with armour and internals, consequences from the closed set,
   components on sections (weapons from the catalogue, sensors, ECM/ECCM, power, one core),
   mobility within the chassis's bounds, heat, silhouettes defaulted from the model's measured
   size. Each field carries its server bound; the validator (D-31-7) runs on every change and lists
   its refusals by field.
3. **Look.** Label, short name, weapon short names, the doll grid (generated from the sections and
   editable), `size_cm` and anchor defaulted from the model.
4. **Compare.** The design beside a shipped unit: armour by facet, weapons by band, speed, heat —
   numbers, not a balance verdict.

Exits: **Export** (the two files, D-31-8) and **Publish** (D-31-9: a new version to the instance,
refused with the server's reason if the instance disagrees with the editor, which is a bug). A
`@unit/design list|show|withdraw` staff command reads the registry from the line client.

## 7. The refit and repair bay — IC hulls only (R4)

**D-31-13 The bay is a front end over work orders, never an edit.** The bay is the same studio
opened on **a hull**, not a design: it shows the hull's current state — each section's armour and
internals, each component's condition (intact, damaged, jammed, destroyed, stripped), ammunition,
operational state — on the doll, and offers only actions that are work orders:

| Action | Engine | What it takes |
|---|---|---|
| Repair a damaged section | `WorkshopBook::repair_section` | tariff cost a structural point, IC time |
| Reconstruct a destroyed section | `reconstruct_section` | base cost and time |
| Repair a damaged or jammed component | `repair_component` | cost, time |
| Strip a destroyed or unwanted component | `strip_component` | title or salvage authority (doc 27 FO-4); the part goes to the bay's stores with its provenance |
| Fit a like-for-like replacement | `replace_component_in_slot` | a part of the slot's definition, from stores (a stripped record) or bought; labour, time |
| Rearm | `rearm_component` | rounds, cost, time |
| **Refit** to another design | `replace_component` between registered roots | the target design is registered and in the hull's refit family; the part; an industry refit order (doc 08) on a facility with the capability |
| Restore everything | `quote_full_restoration`, then the orders it lists | the quote's total |

Every action produces a quote first (parts, cost, IC duration, facility), and nothing happens
until it is accepted and its order runs to completion on the IC clock at a facility that has the
capability (`unit.maintain_refit`, `industry.core.refit` on its scope) — doc 08's driver owns the
recipe, the facility and the duration; the bay never sets a number. A hull in the field gets only
what doc 27's field engineering already allows; the bay is a facility.

**D-31-14 A refit is a change of design.** `replace_component_in_slot` only fits a part of the
slot's own definition: a destroyed 105 mm cannon is replaced by a 105 mm cannon. Fitting a laser in
its place changes what the unit *is* — its rules — and in this tree a unit's rules are its
registered definition, so a refit is `replace_component` from the hull's design to another
registered design that differs only in what the refit changes. The designer is how that second
design exists: staff publish the variants a chassis may be refitted to (the line tank's laser,
thermal, ECM and ECCM variants are already exactly that family), and each design declares its
**refit family** — the designs a hull of it may become in a bay. The bay offers the family, never a
free edit, and `core_replacement.rs`'s adapter shows the shape a family adapter takes. A refit's
look can change with it (a laser tank names its own model when one is baked) — the look is never in
the hash (D-22-1), so that costs nothing in the rules.

Arena and simulator pods have none of this (R4): a match's damage ends with the match, and a pod
has no hull to repair. The bay refuses a non-IC unit by kind.

**Damage on screen.** Per R2, the bay and the cockpit show damage on the doll and the text panel
and, for a destroyed unit, the wreck pose; the sprite stays whole.

## 8. Later: damage on the sprite (R2's route)

Recorded so it is not reinvented. kitty's graphics protocol accepts **frame edits against a placed
image** (`a=f`: a rectangle of new pixels composed onto an existing image, by image id), so a
damaged section can be drawn by patching the placed sprite, not by re-sending it, and PR #154's
retained placements are already images with ids. The shape: the bake renders a per-part ID pass
(the colour-ID pass already separates materials; a part pass separates the assembly's parts in
their mounts, which the config names), the reducer stores per part a mask a frame, and a destroyed
section maps to parts (the look's doll legend already maps sections) whose masks are drawn
scorched or cleared — on the web canvas by compositing, on kitty by `a=f` against the placed
image, in the half-block encoders by redrawing the cell rows. It costs a mask a part a frame (the
runs compress it well) and a section-to-part map on the look. Not built, not scheduled.

## 9. Later

- **Damage visuals** (§8).
- **The bake queue**: a staff request for an assembly not in the catalogue lands as a model config
  in a PR; the bake runs on the owner's Windows box (Unity and the packs are there and nowhere
  else) by hand or by a self-hosted Windows runner; the model ships with the next build.
- **Player designs** (D-22-7): the same studio and validator, ungated, with what staff designs do
  not need — who may design, what a design costs to register and to build (doc 08), who may field
  it, a review step before a design reaches a match, and limits on the registry's growth.
- **A repair-training simulator** (R4): a pod scenario that puts a virtual damaged hull in a
  virtual bay.
- **Live catalogue extension**, if Q-31-1 rules for the restart.

## 10. Questions

- **Q-31-1 When does a published design become fieldable?** The catalogue is frozen at the
  composition root, and three readings of it (the unit catalogue, the registry descriptors, the
  arena applier's bindings) agree only because they are one freeze. **Default: at the next
  mush-server restart** — the gateway's hot restart (M16a–M18) keeps every session — so publishing
  is "publish, then a hot restart of the server half" (the gateway's supervisor relaunching
  mush-server), and the freeze stays an invariant. The alternative, appending to a
  live catalogue between matches, is a change to three owners and doc 16 §9's extension rule; it
  waits for a reason.
- **Q-31-2 May an IC hull be built on a design that exists only in the registry?** **Default:
  yes** — the registry is as durable as a ledger row and the design is checked by the same code as
  shipped content — but a design must be committed to the repo before a release that resets the
  instance's Redis (pre-launch; backup and restore are doc 16c T-112's). The stricter reading (IC only from shipped content;
  the registry for arena and test play) is one line in the hull builder if the owner prefers it.

## 11. Cuts

| Cut | What | Done when |
|---|---|---|
| **UD.0** | this document and its README row | — |
| **UD.1** | the model catalogue (D-31-1..5): bake configs under `scripts/unit-bake/models/`, per-model `.mupk` files and `models/index.json` (format 3, keyed by model id), `"model"` on the six looks, the matrix check a model at a time and the look-against-model check, native binaries embedding every model by build script, the web's fetch by roster and on first sight with the text-sheet fallback (D-31-11), immutable content-named files; `units.bin` deleted | the six U.6 models ship as six files; a web client in a match of tanks fetches the tank's file and no other; a contact whose model has not arrived draws its text sheet and then its bake; no change to any picture once loaded |
| **UD.2** | the staff designer v0 (D-31-6..8, §6): the engine and studio, the chassis templates, the model preview through `mush-pixel-wasm`, the rules and look panes, the validator in wasm, export to the two repo files | staff draw a biped variant with a different weapon fit from the catalogue, the studio refuses a broken one with the server's reason, and the exported files pass `cargo test` unchanged when committed |
| **UD.3** | the runtime design registry (D-31-9, D-31-10): `tacsim:design:` rows on the pack ledger, the design namespace, Publish, `@unit/design`, the catalogue generation on the match record and its replay, Q-31-1 as ruled | a published design is fielded in an arena match after the restart, the match replays bit for bit, and committing the design leaves the catalogue's digests unchanged |
| **UD.4** | the refit and repair bay on IC hulls (D-31-13, D-31-14): the bay view on a hull, quotes, the workshop orders through doc 08's driver at a facility, refit families and the refit adapter, the arena refusal | an IC hull back from a sortie with a destroyed cannon is stripped, refitted with a stored like-for-like part, repaired and rearmed over IC time at a priced quote, or refitted to a family variant; a simulator unit has no bay |

UD.1 stands alone and could follow U.6 directly. UD.2 needs UD.1's index; UD.3 needs UD.2's
export; UD.4 needs UD.3 only for refits to published designs (repair and like-for-like work need
neither, and could go first once IC hulls return damaged, doc 27 H2).

## 12. Decisions

- **R1–R4** the owner's rulings of 2026-10-30 (§1).
- **D-31-1** Models in a catalogue keyed by model id; a look names its model; amends D-22-10's
  keying by look id.
- **D-31-2** One immutable, content-named format-3 file a model, and an index.
- **D-31-3** Native binaries embed every shipped model; the gateway never waits on a file.
- **D-31-4** The catalogue is curated whole assemblies, pre-baked on the owner's box; no
  compositing of parts; a bake queue later.
- **D-31-5** A model's mounts and muzzles bound which designs suit it (a look check, not a rule).
- **D-31-6** A design is a rules pack and a look, composed from closed parts, versioned and never
  re-signed.
- **D-31-7** The server's validators run in the editor through wasm; one implementation.
- **D-31-8** Two exits: export to the repo, publish to the instance; the same bytes.
- **D-31-9** The runtime registry rides the pack ledger, in its own id namespace; withdrawn, never
  deleted.
- **D-31-10** The match hash folds the fielded catalogue as today; a match records its catalogue
  generation and a replay rebuilds it.
- **D-31-11** The web fetches a model by roster at muster and on first sight, text sheet until it
  arrives; nothing else crosses to a player.
- **D-31-12** The editor fetches models on demand; rule 7 becomes a per-model web ceiling.
- **D-31-13** Repair is work orders over doc 27's workshop and doc 08's driver on IC hulls, quoted,
  timed and priced; never a free edit; arena units have no bay.
- **D-31-14** A refit is a change of registered design within the hull's declared refit family.
