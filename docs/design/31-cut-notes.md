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
- **The bake**: configs moved to `scripts/unit-bake/models/<model id>.json` (`look` → `model`);
  `Bake.cs` writes `meta.json` format 3 with `model`; `unit_reduce` writes through
  `model::write_model` (`--models`, removing the model's older file, rewriting `index.json`);
  `bake.sh` bakes every config by default. **Not run here**: `Bake.cs` needs Unity and the packs,
  which are on the owner's machine; its change is the field rename and the meta's format number.

**Not done here, and why.**

- **The roster prefetch** (D-31-11's "at muster"). No feed carries the match's whole roster
  today: the cockpit frame names the pilot's own type and each *classified* contact's. A roster
  field listing every type in the match would tell a client what the other side fielded before its
  sensors did — an epistemic question (the Epistemic Engine's per-observer knowledge) for the
  owner, not a delivery detail. Until it is ruled, the client fetches on first sight: its own
  model with the first frame, a contact's when it is first classified, the `unknown` hump with the
  first unclassified contact; the text sheet draws meanwhile.
- **D-31-5's mounts in the index** (the parts in their mounts and the muzzle locators by weapon,
  for the designer's fit check): the index carries the prefab line; the structured mounts come with
  UD.2, which reads them.
- **Not seen in a live browser**: the fetch path is covered by the wasm and vitest tests (wanted
  files named, installed, none wanted after) and `vite build`; no dev2 session was driven.
