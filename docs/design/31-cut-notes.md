# 31. Cut notes — the unit designer and the refit bay

## UD.1 (2026-10-30) — the model catalogue, D-31-1..4, D-31-11..12

Built on `claude/ud1-model-catalogue` against doc 31 Draft 0.3, one PR. No re-bake: the six U.6
entries were split out of `units.bin` byte for byte (the format-2 and format-3 entry bodies are the
same; only the key's meaning and the version changed), so every picture draws exactly what U.6 drew
— the native goldens and the web's parity hashes are unchanged.

- **The catalogue** (`crates/mush-pixel/models/`, `mush_pixel::model`): one content-named file a
  model, `<id>.<first 12 hex of its SHA-256>.mupk` (a one-entry unit pack, format 3, keyed by model
  id), its hand-drawn fallback sheet `<id>.sheet.json`, and `index.json` (id, file, SHA-256,
  bytes, the baked `size_cm`, the sheet, the asset pack and prefab). The six U.6 assemblies, renamed:
  `mc.lt.fox_biped` (218,899 bytes), `mc.st.tank_lvl2` (204,977), `mc.st.buggy_lvl1` (186,474),
  `mc.st.tank_lvl3` (185,767), `mc.st.spider_flybot` (170,057), `prim.sphere_hump` (77,381):
  1,043,555 bytes for six, 84 more than U.6's one file (five more headers). The ids follow the asset
  kits (`mc.lt` *Light and Medium Robots*, `mc.st` *Spiders and Tanks*, `prim` a Unity primitive).
- **The looks** gain `"model"` and a top-level `"size_cm"` (required); `look::Sheet` loses its
  `size_cm`. The six shipped looks' `sprites` moved to their models' sheet files; `Look::sheet()` is
  the look's own sheet, else its model's — the path a designed look with no sheet will take (UD.3).
  `look::parse_sheet_text` reads a sheet file by the `sprites` rules.
- **The table** (`mush_pixel::bake`): `install` adds a model file's entries the table lacks and is
  callable any number of times (U.6's `OnceLock<Pack>` took one pack); a slot never moves; entries
  are leaked once a model so the pictures keep their `'static` chains. Native builds embed every
  file by build script (`build.rs` lists `models/` into `$OUT_DIR/models.rs`; the sheets in both
  builds, the `.mupk` files only with `encode`), so the gateway never waits (D-31-3).
- **Keyed by model** (Draft 0.3's review point 4): `bake_of_sheet`, `look_of_sheet`, `pack()`,
  `embedded()`, `ROSTER`, `WEB_FETCHES_PACK`, `PACK_DIGEST` and `check_matrix` are gone.
  `SpritePose` carries the look's `size_cm` and `model` (`SpritePose::of_look`), and the raycaster
  and heightfield find the bake by model id. The 2.5D draws an `Art` — a model's table slot or a
  look's sheet — and its kitty sheet cache keys on it (`SheetKey { art, .. }`), so looks on one
  model upload one sheet. The fallback chain cache stays per look, and a draw asks the table first,
  so a model that arrives supersedes the fallback on the next frame.
- **The checks**: `check_model` (the matrix check a model at a time, against every look that names
  it: height within 2 %, width within half to twice, muzzles naming a weapon of one of them, a model
  no look names refused); the catalogue test `model::tests::the_catalogue_holds` replaces the pinned
  pack digest — one file a model, its name its own SHA-256, its length and digest the index's, one
  entry of that model, under rule 7's per-model 512 KiB, every sheet shipped and parsing, every
  look's model in the catalogue; `every_shipped_sheet_stands_moves_and_wrecks` now walks the
  models' sheets too (it would otherwise have skipped all six once they left the looks).
- **The web** (ABI 3): `mp_units` adds one model a call; `mp_wanted` / `mp_output` name the files
  the held feeds need and the module lacks — every `unit_type` anywhere in a held feed (a `null`
  one is an unclassified contact, the `unknown` look) mapped through its look to its model.
  `PixelModule` fetches them after every kept feed (`fetchModelsFrom`), once each, retrying a
  failed fetch on a later feed; `web/scripts/build-wasm.mjs` copies `models/*.mupk` beside the
  module and Vite fingerprints them into `assets/`, which the server already serves
  `immutable`. The page no longer fetches `units.bin`. `vite build`: six files, 77–219 kB, beside a
  750 kB module that embeds none.
- **The own-side roster** (D-31-11, ruled by the owner on PR #196: own side at muster, the other
  side on first sight, *"avoids the info leak"*): the cockpit frame carries `side_unit_types`, the
  distinct sorted unit types of the pilot's own team (`Frame::side_unit_types`, built in
  `frame_for` from the unit table and its chassis words), and the model walker reads it beside
  every `unit_type`. A few dozen bytes on each keyframe; `arena.map` stays a fact about the ground
  with no unit in it. The cockpit fixtures (`cockpit_frame.json`, `view_frame.json`) are re-blessed
  with the key.
- **The bake**: configs moved to `scripts/unit-bake/models/<model id>.json` (`look` → `model`);
  `Bake.cs` writes `meta.json` format 3 with `model`; `unit_reduce` writes through
  `model::write_model` (`--models`, removing the model's older file, rewriting `index.json`);
  `bake.sh` bakes every config by default. **Not run here**: `Bake.cs` needs Unity and the packs,
  which are on the owner's machine; its change is the field rename and the meta's format number.

**Not done here, and why.**

- **D-31-5's mounts in the index** (the parts in their mounts and the muzzle locators by weapon,
  for the designer's fit check): the index carries the prefab line; the structured mounts come with
  UD.2, which reads them.
- **Not seen in a live browser**: the fetch path is covered by the wasm and vitest tests (wanted
  files named, installed, none wanted after) and `vite build`; no dev2 session was driven.

## UD.2 (2026-10-30) — the staff designer v0, D-31-6..8 (D-31-7 amended)

Built on `claude/ud2-staff-designer`, stacked on UD.1 (PR #196). The owner amended D-31-7 at the
start of the cut: the rules are checked by a staff-only server query, the look checks and the
preview stay in wasm (doc 31 §4 says why the schema crate was dropped).

- **The protocol** (`mush-proto`): a third invoke service, `unit_design` —
  `UnitDesignRequest::Check { id, version, rules }` (bounded: an id of at most 96 bytes, a
  version of 1 or more, `rules` an object of at most 128 KiB) and
  `UnitDesignResponse::{Checked { refusals, pack }, Error { code, message }}`; the TS bindings
  regenerated. The SSH TUI's two exhaustive matches on `InvokeResult` take the new arm (the TUI
  never asks).
- **The server** (`tacsim::design`): `check` makes the one-entry pack (`tacsim.pack.<slug>`,
  kind `rules_definition`) holding `tacsim.unit.<slug>`; `requires` is each shipped pack holding an
  entry the `def` names by id (weapons, sensors, cores), in the order first named; the digest is
  the content layer's own (`rules_pack_of(..).computed_digest()`); then `pack::load_pack` and
  `Catalog::build_fielded` with the pack, so every refusal is the loader's or the build's
  sentence. An id outside `tacsim.unit.`, or one a shipped unit or pack already uses, is refused
  before that. The invoke handler is registered beside the hardware in `TacsimRuleset::register`
  and refuses anyone who is not staff (`commands::is_staff`). Tested: the shipped laser tank's
  `def` under a new id holds and comes back with exactly the shipped file's `requires`, and its
  pack loads as a file; an empty section list, an unknown weapon, a stray key, a shipped id and a
  bare id are each refused. A check runs in well under 0.1 s (the two tests, 0.06 s).
- **The look half** (`mush_pixel::design`): `check_look` — doc 22 §6's look-against-unit rules
  plus the look-against-model check (catalogue membership, height within 2 %, width within half to
  twice; a weapon the model shows no muzzle for is a note) — and `preview_rgba` (a model's frame,
  its bake or its fallback sheet's chain, in team colour). `mush-contract-tests`' `unit_looks` now
  holds every shipped look to `check_look`, so the designer and the test are one implementation.
  `mush-pixel-wasm` exports `mp_check_look` and `mp_preview` (the module grows from 750 to 821 KB).
- **The web**: the engine `web/src/lib/designer/unit-designer.ts` (the catalogue as templates, a
  draft, the edits, `History` undo/redo, the `localStorage` draft, the test hit, the export of the
  two files) and the RPC client `rpc.ts`; the studio `UnitDesignerStudio.svelte`, a third tab of
  the Build panel beside the map studio, four panes (chassis and model with the baked preview by
  view, facing and pose; rules — label, top speed, internals and armour by facet, the weapon each
  weapon component fits; look — short and weapon names, the doll, the test hit by clicking a doll
  part; compare and checks). **Check on server** sends the draft; **Export** downloads the
  server's pack and the look once both checks are clean. `build-wasm.mjs` copies the shipped
  unit, weapon, sensor and core packs, the looks and the model index under
  `src/lib/designer/content/` (gitignored) for the templates.

**Not done here, and why.**

- **The tab is shown to everyone** in the Build panel; the server refuses a non-staff check
  (*"The unit designer is for staff."*), so a non-staff user can draft but never get a pack. The
  client has no staff flag to hide it by today.
- **New sections and components** are not in v0's panes: a design starts from a shipped unit and
  edits its numbers, weapons, names, model and doll. Adding or removing a section or a component,
  consequences and sensors/cores are edits of the `def` the server already checks; their panes
  come with the refit families (UD.4), which need the slot-stable mount ids (D-31-14).
- **Not seen in a live browser against a server**: the engine, the RPC client, the studio (jsdom)
  and the wasm exports are tested; the server check is tested in tacsim; no dev2 session was driven.

## UD.3a (2026-10-30) — the in-instance design registry, D-31-9

Built on `claude/ud3-design-registry`, stacked on UD.2 (PR #197). UD.3 is split: this cut makes a
published design fieldable; UD.3b carries the tactics catalogue, the catalogue generation on the
match record and the looks on the feeds.

- **Publish** (`mush-proto`): `UnitDesignRequest::Publish { id, version, rules, look }` (the look
  bounded as the rules are, 128 KiB) and `UnitDesignResponse::Published { id, version,
  refusals }`. `tacsim::design::publish`, staff only: an id outside `tacsim.unit.design.` is
  refused (D-31-9's namespace — a published design never shadows a unit shipped later; Check still
  takes any `tacsim.unit.<name>` for a repo export); a version already in the registry, or already
  on the world's pack ledger under the design's pack id, is refused ("a published version never
  changes"); then the check runs and the row is written.
- **The rows** (`tacsim:design:{id}@{version}`, the TacSim namespace): `DesignRecord { id,
  version, pack, look, withdrawn }`, hydrated by `store::load` into `TacsimStore::designs`, written
  by `store::put_design` alone.
- **The boot** (`mush-server`): `boot::design_sources` lists the namespace's keys and reads the
  design rows before `game_modules`; `config_ext::tacsim_config_with_designs` adds each design's
  pack to the operator packs **only when the whole set still verifies with it**, else skips it with
  a warning naming it — a registry row never fails a boot (a design whose required pack moved
  stops loading; it does not stop the world). Withdrawn designs load (a hull may name one). The
  boot's own pack ledger then records each loaded design as it records any operator pack.
- **A defect fixed on the way**: `tacsim::verify_sources_with` resolved an operator pack's
  `requires` only against the other operator packs and the game seeds, never the compiled-in
  content, so a design — which requires the shipped weapon, sensor and core packs — would have been
  refused at boot. It now resolves against the shipped, fielded packs too (parsed once); a
  configured pack under a shipped pack's id and version stays the one resolved. UD.2's
  `design::check` now runs `verify_sources` itself, so a check can never pass a pack a boot would
  refuse (its test asserts it).
- **`@design/list|show|withdraw`** (staff): `@unit/design` in the doc, renamed because `@unit` is
  the tactics module's verb. Withdrawal sets the flag on every version (or the one named); the
  lobby's `fieldable_ids` leaves out a design all of whose versions are withdrawn, so `SIM UNIT`
  neither lists nor accepts it.
- **The studio**: **Publish** after a clean check; the answer says the design is fieldable after
  the next restart. Drafts now default into the `tacsim.unit.design.` namespace.
- **Tests**: `tacsim/tests/shard_3/designs.rs` — published over the invoke path, refused a second
  time, listed and shown with its `requires`, then a restart over the folded rows with
  `design_sources`' packs puts the design in the frozen catalogue, its row hydrated; withdrawn, it
  stays loaded; an id outside the namespace is refused; the registry is staff-only. `mush-server`:
  a design that will not verify is skipped and a good one kept; the layer off adds none.

**Not done here, and why.**

- **The configured-pack cap** (`MAX_CONFIGURED_PACKS`, 64) counts designs: a world with many
  designs and many pack files reaches it, and the boot refuses past it. Whether designs get their
  own cap is a question for when the registry grows; staff-only publishing keeps it small.
- **The tactics catalogue** (`@unit/create`, IC hulls on a design, Q-31-2): the composition root
  builds `TacticsModule`'s catalogue from the shipped content in `tacsim::composition`, which the
  host's packs do not reach; threading them needs six composition signatures across three crates.
  UD.3b.
- **Not seen live**: no dev2 restart with a published design was driven.

## UD.3b (2026-10-30) — the tactics catalogue and the looks, D-31-9, D-31-16; D-31-10 corrected

Built on `claude/ud3b-catalogue-looks`, stacked on UD.3a (PR #199).

- **The tactics catalogue** (D-31-9): `tacsim::composition::tactics_catalog_with(cfg)` builds the
  fielded catalogue over the host's packs — the operator's and the published designs — and
  `mush_composition::server_modules_with_tacsim_units` hands it to `TacticsModule`; `boot.rs` calls
  that in the ordinary composition (no launch deployment, no mind commerce). `TacsimComposition::Standard`
  carries the optional catalogue; every existing public function keeps its signature (the dozen
  test callers unchanged). The launch and commerce compositions keep the shipped catalogue: each
  builds its tactics module from its own captured resources, and neither is a path a staff
  designer publishes into today.
- **`unit.looks`** (D-31-16, as built differently from Draft 0.3): a **pulled** feed, not a `looks`
  map on every feed. `{"types": [...]}` (at most eight) is answered `{"looks": {type: look}}` for
  each type that is a published design (its newest version's look). A look is presentation, not
  match state, so answering any type a client names leaks nothing; and a pull, asked once a type,
  costs no bytes on the 20 Hz cockpit keyframe. Registered with the designer's invoke, after the
  port's feeds; `boot_parity`'s manifest list carries it.
- **The runtime look table** (`mush_pixel::look`): `install(unit, text)` (parsed and checked as a
  shipped look; a unit with a look keeps it), `for_unit` consults it after the shipped looks,
  `by_index`/`index_for_unit` span both, and `wanted(types)` names the types with no look. The
  2.5D's `look_index` and the bake's `iso_art`/`art_chain` read through `by_index`; the fallback
  chain cache became a map keyed by look index, so an installed look gets one.
- **The web**: `PixelModule.askLooksWith(ask)` asks, after every kept feed, for the types the held
  feeds name with no look, once each, eight a call; `installLooks` takes the answer
  (`mp_wanted_looks`, `mp_looks`). `GameApp.bindPixelModule` sends the asks as `unit.looks`
  queries where the manifest carries the feed and routes the answers back; `Game.svelte` binds the
  page's module (`PictureView` stays inside its architecture test's two app calls).
- **The SSH TUI**: a cockpit, replay or observer keyframe naming a type with no look asks
  `unit.looks` once per connection; the answer installs the look into the gateway process's
  table, so every picture it draws finds it.
- **D-31-10 corrected**: Draft 0.1 misread the arena hash's item 3c as folding the server's unit
  catalogue; it folds the arena's mind-card catalogue, and a unit enters the arena with its rules
  resolved and serialized in the snapshot. So no catalogue generation is recorded and a tape needs
  no rebuild (doc 31 §4).
- **Tests**: `tacsim/tests/shard_3/designs.rs` — the host's tactics catalogue holds the design and
  the shipped one does not; the looks feed answers the design's look and nothing for a shipped type.
  `mush-pixel-wasm`: a feed naming a design asks its look, the answer installs it once, the type is
  drawn from it. `web`: the module asks once and installs (`pixel.test.ts`); the app asks the feed
  with the types and routes the answer, and asks nothing without the feed (`game.test.ts`).
  `mush-net-ssh` (`quarry_screen.rs`, for its helpers): a cockpit frame asks once and the answer
  installs.

**Not done here, and why.**

- **The launch and commerce compositions** keep the shipped tactics catalogue (above).
- **Not seen live**: no dev2 restart drew a published design over SSH, kitty or the web.

## UD.4a (2026-10-30) — the refit engine, D-31-14 and D-31-15 as amended

Built on `claude/ud4-refit-bay` from `main`. UD.4 is split three ways (doc 31 §11): this cut is
the pure engine; the authoritative bay order is UD.4b; the bay view and the industry driver UD.4c.

- **The ruling**: refit families match by mount, not by slot ids (the owner, *"what if the next
  mech has two of them?"*). Nothing is renamed: `"cannon"` has 1,030 uses in 103 files, Rhai unit
  minds among them.
- **`refit_component`** (`mush_tactics::unit::replacement`, beside `replace_component`): from an
  exact registered root to another, remove at most one component and install at most one; when both,
  on the same install point. Owner, unit cells, mobility and rules schema equal; sections equal but
  for the caller's derivable section cells; every untouched component equal on everything but its
  definition reference, which must name the same component (equal payload and references, read from
  the registry) and is moved to the target's; capabilities not sourced by the moved components equal.
  A derived cell on both sides keeps its value inside the new bounds; one only the target has starts
  at its `init`. The journal (`RefitJournal`) keeps the removed component's state for custody. Pure:
  no custody, availability, facility or economic authority, nothing written.
- **Found on the way**: a component's definition is unit-scoped (`tacsim.unit.line_tank.machine_gun`
  vs `tacsim.unit.line_tank_laser.machine_gun`), so no component is ever equal across two units;
  the definition-equivalence rule above is what makes a cross-variant refit possible at all.
- **`tacsim::refit`**: `DERIVED_CELLS` (the heat pair), `registry_of(catalog)`, `family(catalog,
  registry, unit)` — every unit in the catalogue one admitted transition reaches, with what comes
  out and what goes in — `fresh_component` (a new part, intact, cells at their starting values) and
  `refit(..)`.
- **Tests** (`tacsim::refit::tests`, on the shipped fielded content): the line tank's family holds
  the laser variant (cannon out, laser in) and the ECM variant (a pod on), not the walker; tank →
  laser adds the hull's heat pair and laser → tank drops it; the pod comes off again; armour damage
  set by a real `SetCell` plan survives a refit of the turret weapon.

**Not done here, and why.**

- **The bay order (UD.4b)**: an IC hull is written only through the tactics IC custody participant
  (`mush_tactics::ic_custody`, the fixed four-owner provider), whose close vocabulary admits cell and
  condition changes and no change of definition. A refit or repair order is a new authoritative
  action there, with its pending state on the IC clock — its own design pass.
- **The bay view and the industry driver (UD.4c)**.
