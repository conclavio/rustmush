# 31. The unit designer and the refit bay

**Draft 0.7 — 2026-10-30, UD.4 split; UD.4a built; refit families found by mount.** The owner
ruled refit families match by mount, not by renamed slot ids (D-31-14 amended). UD.4a is the
engine; UD.4b is the authoritative bay order (the IC custody participant does not yet close a
change of definition); UD.4c the bay view and the industry driver (§11).

**Draft 0.6 — 2026-10-30, UD.3b built; D-31-10 corrected.** The tactics catalogue is built from the
host's packs, and a published design's look reaches the web and the SSH TUI through a pulled
`unit.looks` feed rather than a map on every keyframe (D-31-16). D-31-10 is corrected: the arena
hash folds the arena's own mind-card catalogue and a match carries its units' resolved rules, so no
catalogue generation is recorded ([cut notes](31-cut-notes.md) "UD.3b").

**Draft 0.5 — 2026-10-30, UD.3 split; UD.3a built.** UD.3a makes a published design fieldable
after a restart; the tactics catalogue, the catalogue generation and the looks on the feeds are
UD.3b (§11). The staff verb is `@design`, not `@unit/design` (`@unit` is the tactics module's).
Building it found operator packs' `requires` resolved without the shipped content, which would have
refused every design at boot; fixed ([cut notes](31-cut-notes.md) "UD.3a").

**Draft 0.4 — 2026-10-30, D-31-7 amended in cut UD.2; UD.1's roster ruled.** Building UD.2
found Draft 0.3's schema crate a tacsim refactor of about 1,500 lines with arena types in tow, and
`mush-core` not building for `wasm32`; the owner ruled the rules be checked by a staff-only server
query instead, the look checks and the preview staying in the editor (D-31-7). The owner also
ruled the muster roster on PR #196: the pilot's own side only, the other side on first sight
(D-31-11). UD.1 and UD.2 are built ([cut notes](31-cut-notes.md)).

**Draft 0.3 — 2026-10-30, the PR #191 review folded.** A review against `origin/main` (PR #188
merged) found six gaps, each confirmed in the code, and proposed three rulings, two taken whole and
one in part (the owner: *"fold your edits into pr 191"*). The refit engine refuses the line tank
family as the tanks are written, so refit families need slot-stable component ids and a refit
transition that `replace_component` is not (D-31-14, D-31-15); a published look has no way to the
gateway or the browser, so the feeds carry the looks a match uses that are not shipped (D-31-16);
the host reads the design rows before the module registers, and the tactics module's own unit
catalogue is fed from the fielded one (D-31-9); `size_cm` moves to the look's top level and the
text-sheet fallback to the model, and bakes are keyed by model, not by sheet pointer or roster
index (D-31-1); the installed pack becomes a table that grows a model at a time (D-31-2); the bay's
capability is `unit.maintain_refit` alone and its orders need a pending state on the IC clock
(D-31-13). The validator moves into a small schema crate rather than pulling tacsim into wasm
(D-31-7); a catalogue generation is a count over an append-only list (D-31-10); the studio stamps
the pack's digest and `requires`, keeps drafts and undo, and previews hits (§6).

**Draft 0.2 — 2026-10-30, the questions ruled.** The owner ruled both questions on PR #191 the
same day: Q-31-1 at the restart for now, with live catalogue extension deferred for its cost and
made a prerequisite of player designs; Q-31-2 at the default. The document carries no open
question (§10).

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
bake of six looks, 1,043,471 bytes. PR #188 merged on 2026-10-30; "as built" now means `main`.

## 1. The owner's rulings (2026-10-30)

- **R1 — the model/look split: yes.** (*"sounds good to me"*.) A **model** is one baked assembly
  — the art; a **look** is how one unit type is presented — its label, short name, weapon names,
  doll, `size_cm`, and the model it is drawn from. Models live in a catalogue keyed by
  **model id**; a look names its model. This amends D-22-10, which keyed the baked chains by look
  id (D-31-1).
- **R2 — sprites stay whole under damage, for now.** Damage shows on the doll (doc 22 §3.1) and,
  at the end, in the wreck pose; a unit that has lost its arm still draws with its arm. The owner:
  *"kitty accepts patches against images, so there's a line of sight to doing it reasonably
  cleanly, but not required for where we are at right now."* The later route is recorded in §8.
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
its own mind-card catalogue (`arena/hash.rs`, item 3c; corrected in UD.3b, D-31-10 — not the
server's unit catalogue), and a persisted unit names its exact `DefinitionRef` digest and fails boot if that definition is gone.

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
look split. The matrix check holds a look to its model's measured height and width (doc 22 §6, the
width rule) instead of a chain to its look.

Three things in `mush-pixel` move with the split (Draft 0.3, the review):

- **`size_cm` leaves the sheet.** Today `size_cm` and the anchor both live inside `sprites`
  (`look::Sheet`), so a look without a hand-drawn sheet has neither. `size_cm` moves to the look's
  top level (every look has one; the sheet's copy goes); the anchor stays with whatever is drawn —
  a text sheet has its own, and every baked frame already carries its own per size.
- **The fallback belongs to the model.** A designed look has no text sheet of its own; what it
  draws while its model is in flight, or in a picture with no baked chain, is its **model's**
  fallback sheet — a hand-drawn 96 px text sheet beside the model's file (the six U.6 looks' sheets
  become their models' sheets), or `unknown`'s. A look may still carry its own sheet; a hand-drawn
  unit with no model is exactly that.
- **Bakes are keyed by model id.** `bake_of_sheet` finds a bake by the sheet's pointer among
  `look::shipped()`, and `iso25d` indexes bakes and fallbacks by the look's position in that list
  (`look_index`); both become lookups by model id, which is also what kitty's uploaded-sheet cache
  (`SheetKey`) keys on, so five tank variants on one model upload one sheet, not five.

**D-31-2 Per-model files.** The single `units.bin` becomes one file a model,
`crates/mush-pixel/models/<model id>.<digest prefix>.mupk`: the format-2 entry unchanged in body,
keyed by model id, its name carrying the first twelve hex digits of its SHA-256 so a client can
cache it forever and a re-bake is a new name. A `models/index.json` lists id, digest, size, height
and the bake's provenance line. The format version moves to 3 for the key change; the pack's codec
and rule 7's runs do not. The matrix check runs a model at a time.

The installed pack becomes a table that grows (Draft 0.3): today `bake::install` fills a
`OnceLock<Pack>` and a second call drops its bytes, and `iso25d`'s `FALLBACK` caches a look's
fallback chain on first draw for good. Both become a model table keyed by model id that accepts one
model's file at a time (a lock on native, a cell on wasm), and a draw asks the table before the
fallback, so a model that arrives mid-match supersedes the fallback on the next frame.

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

**D-31-7 One validator — the rules on the server, the look in the editor** (amended 2026-10-30
in cut UD.2; the owner: *"Use a staff-only server check query for the rules; keep the look checks
and preview in wasm. Amend D-31-7 in the doc."*). Every check a design meets is run by the code the
game itself runs, never by a second implementation:

- **The rules are checked on the server.** The studio sends the design — its unit id, version and
  `def` — as a staff-only `unit_design` invoke (`mush_proto::UnitDesignRequest`; refused for
  anyone not staff). `tacsim::design::check` makes the one-entry pack the design would ship as,
  **computes its manifest** — the `requires` (each shipped pack carrying a weapon, sensor or core
  the `def` names) and the content layer's own `digest` — and runs it through `pack::load_pack`
  and `Catalog::build_fielded`: every bound, every cross-check and the binding to a tactical unit,
  as a boot would. The answer is the loader's or the build's own sentence, or the finished pack,
  ready to commit; nothing is stored. A check takes well under a tenth of a second.
- **The look is checked in the editor**, by `mush_pixel::design::check_look` in
  `mush-pixel-wasm` (`mp_check_look`): every section of the unit in the doll, every doll part a
  section and facet the unit has, every weapon name and sheet muzzle a weapon component of it, the
  model in the catalogue and its height and width within the matrix check's bounds of the look's
  `size_cm` (D-31-5; a weapon the model shows no muzzle for is a note, not a refusal). The
  `unit_looks` contract test holds every shipped look to the same function. The model preview
  (`mp_preview`) draws in the editor too.

Why not a schema crate in the browser (Draft 0.3's plan): the unit types are tied to the arena —
`UnitSpec` and its checks read `TrackedRules` and `Side` (`arena::state`), `MAX_SECTIONS`
(`arena::condition`), `CostRow`, `HULL_SECTION` and the logistics, interior and stowage
definitions — so the move was about 1,500 lines of tacsim with arena types in tow, against
branches other sessions were working in; and `mush-core`, which the ids come from, does not build
for `wasm32` (its request epoch reads `getrandom`). The server check costs a live connection, which
publishing needs anyway.

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

**Where the rows enter the boot** (Draft 0.3, the review). `TacsimRuleset::register` builds the
catalogue and registers every descriptor, and the engine freezes the definition registry before
`on_world_loaded` runs; a persisted unit on a design the registry lacks then fails its exact
definition check during hydration. So the host reads every `tacsim:design:*` row from Redis
**before** the tacsim module registers, beside the files it reads for `[tacsim].pack_paths`, and
hands both to the one loader as operator packs. One more reading must join the freeze: the tactics
module's `UnitCatalog` (behind `@unit/create`) is built by `tactics_catalog()` from the shipped
content alone, so today not even an operator pack reaches it; UD.3 feeds it from the fielded
catalogue, which mends that for operator packs too.

**D-31-10 A match carries its own rules; no generation is recorded** (corrected 2026-10-30 in cut
UD.3b). Drafts 0.1–0.3 read the arena's hash item 3c as folding the **server's** unit catalogue, so
that a published design would move the hash of every later match and a match would have to record
the catalogue generation it ran under. It does not: item 3c folds the arena's own **mind-card**
catalogue (`arena::runtime_card::CardCatalog`, the Rhai profiles admitted for that match), and a
unit enters the arena as a custody unit with its rules already resolved — its tactical unit, its
mobility, sensor, weapons and profile — which the opening snapshot serializes with the rest of the
arena. So publishing a design moves the hash of no match that does not field it; a match that
fields one carries the design's rules in its own snapshot; and the private input tape
(`arena/input_tape.rs`) re-executes from that snapshot without the registry. The public picture
replay (`arena.replay`) replays recorded frames and needs only the looks (D-31-16), which the
registry keeps: a design's versions are withdrawn, never deleted (D-31-9), and never leave Redis
(backup and restore of the instance are doc 16c T-112's; designs are now part of what it must
keep).

A published design becomes fieldable at the next server restart (Q-31-1, ruled, §10).

## 5. Delivery — the overhead is disk and editor bandwidth

The owner's bar: *the only overhead should be disk space and editor bandwidth*. A player's client
pays for the models it draws and nothing else.

**D-31-11 The web fetches a model when it first needs it.** The page no longer fetches
`units.bin`. Each frame's contacts carry a `unit_type`; the client maps type → look → model id →
file name (the look table and the model index ship with the module, a few kilobytes) and fetches
`/units/<model id>.<digest>.mupk` the first time it needs that model, handing it to `mp_units` to
install beside the others. Two triggers: at muster, the pilot's **own side's** types (the cockpit
frame's `side_unit_types`) are prefetched, so the pilot's own side draws baked from the first
frame; and lazily, any type that first appears in a frame (a contact as it is classified, a late
join, a spectator) is fetched then. **Ruled 2026-10-30 (UD.1):** the owner chose own side only —
*"i think its okay to send on first sight for bad guys, avoids the info leak"*: a roster of the
other side would tell a client what was fielded against it before its sensors did. Until a model arrives the contact draws its look's text sheet — the
fallback every picture already has — so a slow fetch costs crispness for a second, never a blank.
The files are immutable and content-named, so they carry a year's `Cache-Control` and a client
fetches each model once per bake, ever. A design published in the instance has no new art (its
model is shipped, D-31-4), so publishing never sends a model to anyone; it sends a look, a few
hundred bytes (D-31-16).

**D-31-16 The feeds carry the looks a match uses that are not shipped** (Draft 0.3, the review).
`look::shipped()` is compiled in, and the panel feeds (`arena.cockpit`, `arena.map`,
`arena.replay`) name a unit's type by id and carry no look; the gateway (SSH, kitty, the text
cockpit) and the browser both draw from `shipped()`, and the gateway half never reads Redis. So the
feeds that open a match — the cockpit's first frame, the map, the replay's header — carry a `looks`
map, unit type to look JSON, for every type in the match whose look is not shipped (with no
`sprites`: a designed look is its label, short name, weapon names, doll, `size_cm` and model, a few
hundred bytes); later frames add a type only when it first appears. `mush_pixel::look` gains a
runtime table beside `shipped()` (`install_look`), filled by the gateway from the feed and by
`mush-pixel-wasm` from the same JSON, consulted after the shipped looks. The look stays out of
every hash (D-22-1); the feed carries it because the drawing side has no other way to learn it.

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
   size. The look check (D-31-7) runs on every change; the rules check runs on the server when
   staff press **Check on server**, and lists the loader's or the build's refusals.
3. **Look.** Label, short name, weapon short names, the doll grid (generated from the sections and
   editable), `size_cm` and anchor defaulted from the model.
4. **Compare.** The design beside a shipped unit: armour by facet, weapons by band, speed, heat —
   numbers, not a balance verdict.

Two working habits from the map studio (Draft 0.3, the review): a bounded undo/redo stack over the
engine's state, and the working draft saved to `localStorage` beside the JSON download and upload,
so a closed tab costs nothing. And one check the server cannot give: a **test hit** in the preview —
click a section or facet, choose damaged or destroyed, and the doll recolours and the consequences
it would trigger (`remove_weapons`, `slow`, `disable_motion`, …) are listed — so an authoring
mistake in a consequence shows before a match finds it.

Exits: **Export** (the two files, D-31-8) and **Publish** (D-31-9: a new version to the instance,
refused with the server's reason if the instance disagrees with the editor, which is a bug). A
`@design/list|show|withdraw` staff command reads the registry from the line client (UD.3a:
`@design`, not `@unit/design` — `@unit` is the tactics module's verb, and a module cannot add a
switch to another's).

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
| **Refit** to another design | the refit transition between registered roots (D-31-15) | the target design is registered and in the hull's refit family; the part; an industry refit order (doc 08) on a facility with the capability |
| Restore everything | `quote_full_restoration`, then the orders it lists | the quote's total |

Every action produces a quote first (parts, cost, IC duration, facility), and nothing happens
until it is accepted and its order runs to completion on the IC clock at a facility that has the
capability (`unit.maintain_refit` on its scope) — doc 08's driver owns the recipe, the facility
and the duration; the bay never sets a number. (`industry.core.refit` is not a bay capability: it
is the governed-hardware licence for fitting a mind core, `RefitKind`'s, and a bay that also refits
cores holds both.) A hull in the field gets only what doc 27's field engineering already allows;
the bay is a facility.

**Orders take time** (Draft 0.3, the review). Every `WorkshopBook` method applies its mutation
immediately and stamps `completed_at_ic` with the time it was called. UD.4 adds the pending state
around it: an accepted order holds the hull in the bay (no launch, no custody move, no second order
on the same section or component) and keeps its parts reserved; at its IC completion time — doc
27's `W-clock` freezes and outage adjustments apply — the driver commits the workshop mutation (or
the refit transition) with the order's own completion time, or refuses it with the parts returned
if the hull's revision moved. Cancelling before completion returns the parts and refunds by the
driver's rule.

**D-31-14 A refit is a change of design.** `replace_component_in_slot` only fits a part of the
slot's own definition: a destroyed 105 mm cannon is replaced by a 105 mm cannon. Fitting a laser in
its place changes what the unit *is* — its rules — and in this tree a unit's rules are its
registered definition, so a refit moves the hull from its design to another registered design. The
designer is how that second design exists: staff publish the variants a chassis may be refitted
to, and each design declares its **refit family** — the designs a hull of it may become in a bay.
The bay offers the family, never a free edit. A refit's look can change with it (a laser tank names
its own model when one is baked) — the look is never in the hash (D-22-1), so that costs nothing in
the rules.

**The family is found by mount, not declared** (amended 2026-10-30 in cut UD.4a; the owner:
*"main_gun kinda sucks as a descriptor because what if the next mech has two of them? … match by
mount is the right path"*). Draft 0.3 had refit families keep one component id a mount, which would
have renamed the line tank family's `cannon` and `laser` — 1,030 uses in 103 files, Rhai minds
among them — and would not have said what to call a second gun. Instead what comes out and what
goes in share a **mount** (the same install point) and each keeps its own variant's id: the cannon
comes out of the turret and the laser goes in, exactly as the shipped variants already have them.
And a hull's family is not declared: it is **every registered unit one admitted transition reaches**
(`tacsim::refit::family`), so a published design joins a family by being built that way, and the
bay offers exactly what the transition admits.

**D-31-15 The refit transition.** A new pure function beside `replace_component` in
`mush_tactics::unit::replacement` (the shape `core_replacement.rs`'s finite adapter already takes):
from an exact registered root to another in its family, it **removes** at most one component and
**installs** at most one (either side optional, so a pod can come off as well as go on), and admits
exactly the section-cell changes the binder derives from the two roots (the heat pair when one has
`heat_max` and the other not, its bounds when both do); every other field must still be equal. The
removed component's state goes to custody in the journal as `replace_component`'s does, and the
heat cell's current value is carried when both roots have it and starts at zero when it is new.

As built (UD.4a, `refit_component`): the derivable section cells are the caller's (TacSim passes
its heat pair), since the tactics crate knows no ruleset's cell names. And a component's definition
is **scoped to its unit** (`tacsim.unit.line_tank.machine_gun`), so the same machine gun on two
variants is two references: an untouched component matches its counterpart on everything but the
reference, the two definitions must be the same component (equal payload and equal references,
read from the registry), and its provenance follows the hull to the target root.

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
- **Player designs** (D-22-7): the same studio and validator, ungated, on live catalogue extension
  (Q-31-1: a player cannot wait for a staff restart), with what staff designs do not need — who may design, what a design costs to register and to build (doc 08), who may field
  it, a review step before a design reaches a match, and limits on the registry's growth.
- **A repair-training simulator** (R4): a pod scenario that puts a virtual damaged hull in a
  virtual bay.
- **Live catalogue extension** (Q-31-1, deferred for cost): a published design fieldable without a
  restart. Required before player designs.

## 10. Questions

**Both ruled 2026-10-30** (Draft 0.2, the owner on PR #191).

- **Q-31-1 When does a published design become fieldable?** **Ruled: at the next mush-server
  restart, for now.** The catalogue is frozen at the composition root, and three readings of it (the
  unit catalogue, the registry descriptors, the arena applier's bindings) agree only because they
  are one freeze; the gateway's hot restart (M16a–M18) keeps every session, so publishing is
  "publish, then a hot restart of the server half" (the gateway's supervisor relaunching
  mush-server). The owner: live publishing is **deferred for its cost, not because it is
  uninteresting** — *"we'll need live catalogue modifications to support player-based designs but
  for now with them being staff-only its temporarily acceptable to require the restart"*. Live
  catalogue extension (appending to the fielded catalogue between matches: a change to the three
  readings' owners and to doc 16 §9's extension rule) is therefore a **prerequisite of player
  designs** (§9), not an optional extra.
- **Q-31-2 May an IC hull be built on a design that exists only in the registry?** **Ruled at the
  default: yes** — the registry is as durable as a ledger row and the design is checked by the same
  code as shipped content — but a design must be committed to the repo before a release that resets
  the instance's Redis (pre-launch; backup and restore are doc 16c T-112's).

## 11. Cuts

| Cut | What | Done when |
|---|---|---|
| **UD.0** | this document and its README row | — |
| **UD.1** (built 2026-10-30, [cut notes](31-cut-notes.md)) | the model catalogue (D-31-1..5): bake configs under `scripts/unit-bake/models/`, per-model `.mupk` files and `models/index.json` (format 3, keyed by model id), `"model"` on the six looks, `size_cm` on the look's top level, each model's fallback sheet, bakes keyed by model id (`bake_of_sheet`, `look_index` and the look-keyed `SheetKey` go), the installed pack and the fallback cache as tables that grow, the matrix check a model at a time and the look-against-model check, native binaries embedding every model by build script, the web's fetch by roster and on first sight with the text-sheet fallback (D-31-11), immutable content-named files; `units.bin` deleted | the six U.6 models ship as six files; a web client in a match of tanks fetches the tank's file and no other; a contact whose model has not arrived draws its text sheet and then its bake; no change to any picture once loaded |
| **UD.2** (built 2026-10-30, [cut notes](31-cut-notes.md)) | the staff designer v0 (D-31-6..8, §6): the `unit_design` invoke and `tacsim::design` (the rules check, the manifest's digest and `requires`), the look check and model preview in `mush-pixel-wasm`, the engine and studio, the chassis templates, the rules and look panes, undo/redo, the saved draft, the test hit, export to the two repo files | staff draw a biped variant with a different weapon fit from the catalogue, the studio refuses a broken one with the server's reason, and the exported files pass `cargo test` unchanged when committed |
| **UD.3a** (built 2026-10-30, [cut notes](31-cut-notes.md)) | the runtime design registry (D-31-9): Publish (the `unit_design` invoke's second op), `tacsim:design:` rows, the design namespace, the rows read by the host before the module registers and joined to the operator packs (a design that no longer verifies skipped, never a failed boot), `requires` resolved against the shipped content, `@design/list|show|withdraw`, a withdrawn design never offered to a new match, the studio's Publish, fieldable after a restart (Q-31-1) | a design published from the studio is in the catalogue after the restart and offered by `SIM UNIT`; the same version is refused a second time; a withdrawn one stays loaded and is no longer offered |
| **UD.3b** (built 2026-10-30, [cut notes](31-cut-notes.md)) | the rest of D-31-9 and D-31-16: the tactics module's catalogue built from the host's packs (`@unit/create` and IC hulls on a design), the pulled `unit.looks` feed, the runtime look table in `mush-pixel`, the web and the SSH TUI asking for a design's look once and drawing from it; D-31-10 corrected (no generation is recorded) | `@unit/create` resolves a published design after the restart; a client seeing a design's type asks its look once and draws its label, doll and model |
| **UD.4a** (built 2026-10-30, [cut notes](31-cut-notes.md)) | the refit engine (D-31-14, D-31-15 as amended): `refit_component` in `mush_tactics::unit::replacement`, the family found by mount (`tacsim::refit::family`), the adapter (`tacsim::refit::refit`) | the shipped line tank refits to its laser variant and back (the heat pair added and dropped) and takes and sheds its ECM pod; a walker is no refit of a tank; damage on the hull survives a refit |
| **UD.4b** (in progress, [cut notes](31-cut-notes.md): PR-A the `component_refit` Tactics participant; PR-B the bay flow and the sibling `bay_refit` kernel protocol; then Economy and Inventory, then TacSim) | the bay order (D-31-13): an authoritative refit action on an IC hull on the full core-refit pipeline (industry work order, economy payment, a part from an inventory lot, the tactics participant), launch-only like cores; the pending state on the IC clock, the hull held in the bay, quotes from `WorkshopFacilityTariff`, a `unit.maintain_refit` facility, the arena refusal | an IC hull back from a sortie with a destroyed cannon is refitted with a stored part, repaired and rearmed over IC time at a priced quote, or refitted to the laser variant; a hull in the bay cannot launch; a simulator unit has no bay |
| **UD.4c** | the bay view in the studio; cost and capacity through doc 08's industry work orders | staff and owners see the hull's state, its family and a quote, and place an order |

UD.1 stands alone and could follow U.6 directly. UD.2 needs UD.1's index; UD.3 needs UD.2's
export; UD.4 needs UD.3 only for refits to published designs (repair and like-for-like work need
neither, and could go first once IC hulls return damaged, doc 27 H2).

## 12. Decisions

- **R1–R4** the owner's rulings of 2026-10-30 (§1).
- **D-31-1** Models in a catalogue keyed by model id; a look names its model; amends D-22-10's
  keying by look id. `size_cm` on the look, the fallback sheet on the model, bakes keyed by model
  (Draft 0.3).
- **D-31-2** One immutable, content-named format-3 file a model, and an index; installed into a
  table that grows (Draft 0.3).
- **D-31-3** Native binaries embed every shipped model; the gateway never waits on a file.
- **D-31-4** The catalogue is curated whole assemblies, pre-baked on the owner's box; no
  compositing of parts; a bake queue later.
- **D-31-5** A model's mounts and muzzles bound which designs suit it (a look check, not a rule).
- **D-31-6** A design is a rules pack and a look, composed from closed parts, versioned and never
  re-signed.
- **D-31-7** (amended 2026-10-30, cut UD.2) The rules are checked by a staff-only `unit_design`
  invoke running the server's own loader and catalogue build, which also computes the pack's
  `requires` and `digest`; the look checks and the preview run in the editor through wasm, one
  function shared with the contract test.
- **D-31-8** Two exits: export to the repo, publish to the instance; the same bytes.
- **D-31-9** The runtime registry rides the pack ledger, in its own id namespace; withdrawn, never
  deleted; read by the host before the module registers, and the tactics catalogue fed from the
  fielded one (Draft 0.3).
- **D-31-10** (corrected in UD.3b) A match carries its units' resolved rules in its own snapshot,
  and the arena hash folds its own mind-card catalogue, not the server's unit catalogue: no
  generation is recorded and a tape needs no rebuild; the registry keeps every version's look.
- **D-31-11** The web fetches a model by its own side's roster at muster (ruled 2026-10-30: never
  the other side's) and on first sight, text sheet until it
  arrives; nothing else crosses to a player.
- **D-31-12** The editor fetches models on demand; rule 7 becomes a per-model web ceiling.
- **D-31-13** Repair is work orders over doc 27's workshop and doc 08's driver on IC hulls, quoted,
  timed and priced; never a free edit; arena units have no bay; `unit.maintain_refit` only; orders
  pend on the IC clock and hold the hull (Draft 0.3).
- **D-31-14** A refit is a change of registered design within the hull's refit family (amended in
  UD.4a: the family is every unit one admitted transition reaches, matched by mount; no ids are
  renamed).
- **D-31-15** (Draft 0.3) The refit transition: remove and install at most one component each, and
  the binder's derived section cells, between family roots.
- **D-31-16** (Draft 0.3) The feeds carry the looks a match uses that are not shipped; a runtime
  look table on the gateway and in wasm.
- **Q-31-1** (ruled 2026-10-30) A published design is fieldable after the next server restart;
  live catalogue extension is deferred for cost and is a prerequisite of player designs.
- **Q-31-2** (ruled 2026-10-30) IC hulls may be built on registry-only designs; commit before any
  Redis reset.
