# 30 — cut notes

One `## WG.n` heading per cut of [30-world-generator-v2.md](30-world-generator-v2.md), in the order they land: what landed, what it deferred and why, how it was verified.

## WG.0

The document ([30-world-generator-v2.md](30-world-generator-v2.md), Draft 0.1 → Draft 0.2 on 2026-10-30, #183) and its `docs/design/README.md` row. Read the week's five terrain pieces (#154, the Unity extractor, #169, #173, #178) as one research spike; decided D-30-1..10 and ruled Q-30-1..9.

## WG.1

`tacsim::ground`, `ground-pack`, the checked-in `proving_ground.draft.json`, and the two-half content pin.

- **`tacsim::ground`** (`crates/tacsim/src/ground.rs`):
  - `GroundDraft` (`ground.draft.v1`, D-30-3), `GroundProvenance` and `GroundSource` (`generator | extractor | studio`), with strict validation (`GroundDraft::validate`): checks schema, canonical ID derivations, biome vocabulary, grid bounds (`<= 16` quadrants of `128 × 128`), provenance (`Extractor` requires non-empty `inputs`), the 3-byte `[cols u16 LE][rows u16 LE] + [count u16 LE][class u8]` class RLE (`encode_classes` / `decode_classes`, refusing code `0` `Void`), `le16` elevations (`encode_elev` / `decode_elev`), and `ArenaDef::check`.
  - Moved the proving ground generator (`class_at`, `elev_at`, `elev_v3`, `Relief`, `atlas_pack_at`, `atlas_pack_v3`, `atlas_pack_with`, `sign`) from `crates/tacsim/tests/support/ground.rs` into `tacsim::ground`, leaving `tests/support/ground.rs` as a `pub use tacsim::ground::*;` re-export so existing test binaries keep their path.
  - `generate("proving_ground", seed, biome)` emits the proving ground's `GroundDraft`.
  - `build(&draft)` validates and checks the draft, quadrants `classes` and `elev` into `128 × 128` `field.q{x}_{y}` heightfield entries, and produces both signed packs (`tacsim.<id>` and `tacsim.pack.<id>`) and their digests.
  - `check(&draft)` validates the draft, prices its cells from `LocomotionTable::shipped()`, and verifies the widest `arena.map` frame size against `MAP_MAX_BYTES` (256 KiB).
  - `from_packs(atlas_json, arena_json)` implements the inverse (`pack → draft`), stitching quadrant heightfields and classes back into a whole `GroundDraft`.
- **`ground-pack` CLI** (`crates/tacsim/src/bin/ground-pack.rs`): `build`, `check`, `gen`, and `from-packs` subcommands, with crate-root panic-surface restriction lints denied.
- **Checked-in draft and two-half pin** (`crates/tacsim/content/ground/proving_ground.draft.json`, `crates/tacsim/tests/shard_2/content.rs`):
  - Replaced `the_shipped_atlas_pack_is_this_generators_output` with `the_shipped_draft_is_this_generators_output` (`preset → draft`) and `every_shipped_ground_pack_is_the_writers_output_over_its_draft` (`draft → packs`).
  - Both shipped packs (`content/atlas/proving_ground.json` and `content/arenas/proving_ground.json`) and their pinned digests (`c7202e25…` and `74f4bd85…`) are reproduced byte for byte without modification.

## WG.2

Picture hygiene in `mush-pixel`: retire `synthesize_relief`, exact-match biome names, and render `rock` as a surface material rather than an extruded wall block.

- Removed `Terrain::synthesize_relief` from [`crates/mush-pixel/src/heightfield.rs`](../../crates/mush-pixel/src/heightfield.rs) and the `procedural_wanted` / `all_flat` fallback from `terrain_from_feed` in [`crates/mush-pixel/src/cockpit.rs`](../../crates/mush-pixel/src/cockpit.rs): no `mush-pixel` path writes an `elev_cm` the `arena.map` feed did not carry (D-30-1, D-30-6).
- Tightened `Biome::from_name` in [`crates/mush-pixel/src/raycast.rs`](../../crates/mush-pixel/src/raycast.rs) and `map_from_feed` in [`crates/mush-pixel/src/cockpit.rs`](../../crates/mush-pixel/src/cockpit.rs) to an exact match on the four biome words (`temperate | mars | lunar | ice`), retiring `surface` substring and legend-word sniffing (D-30-5).
- Implemented D-30-10 (`rock` as a surface material, not an extruded `Wall` block): added `Ground::Rock` in [`crates/mush-pixel/src/raycast.rs`](../../crates/mush-pixel/src/raycast.rs) so `"rock"` and `"cliff"` render with distinct mineral surface shading and zero obstacle height, removed `"building"` from `Ground::from_legend_name`'s `Ground::Wall` arm, and retired the `is_rock_spire` woods-as-spires branch on Mars/Lunar in [`crates/mush-pixel/src/heightfield.rs`](../../crates/mush-pixel/src/heightfield.rs) and [`crates/mush-pixel/src/iso25d.rs`](../../crates/mush-pixel/src/iso25d.rs).
- Added `a_flat_field_draws_flat_under_every_biome` and `a_rock_cell_at_elev_cm_0_blocks_no_sight_line_in_any_picture` to [`crates/mush-contract-tests/tests/pixel_relief.rs`](../../crates/mush-contract-tests/tests/pixel_relief.rs), and updated the 4-biome × 3-view distinctness test in [`crates/mush-pixel/src/iso25d.rs`](../../crates/mush-pixel/src/iso25d.rs) to supply explicit `elev` data.

## WG.3

Sim biome on `RegionDef` and `ground-pack check` / `route`.

- Added `pub const BIOMES: [&str; 4] = ["temperate", "mars", "lunar", "ice"]` and `biome: Option<String>` to [`RegionDef`](../../crates/mush-atlas/src/def.rs) and [`RegionRec`](../../crates/mush-atlas/src/record.rs), validated in `RegionRec::validate` and `mush_atlas::validate` (D-30-5).
- Added `biome: SmolStr` (`biome()`, `with_biome(...)`) to [`ArenaTerrain`](../../crates/tacsim/src/arena/state.rs), populated from `RegionRec::biome_str()` in `ArenaDef::from_pack`, and emitted as `"biome": terrain.biome()` on the `arena.map` frame in [`crates/tacsim/src/match_/cockpit/frames.rs`](../../crates/tacsim/src/match_/cockpit/frames.rs).
- Removed `ArenaDef.biome`, `ArenaDef.procedural_relief`, `ArenaDef::biome_variant`, `SIN64`, and `apply_biome_and_relief` from [`crates/tacsim/src/definitions/arena.rs`](../../crates/tacsim/src/definitions/arena.rs) (D-30-1, D-30-5).
- Extended `CheckReport` and `check(&GroundDraft)` in [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs) with the full §5 grade and passability report: tracked / wheeled / legged passability shares (`locomotion.json` rules), zone-to-zone and key-spot reachability, waypoint graph reachability, max cell and road grade (`%`), entry-zone step check, and failure list.
- Added `relax_slopes(&mut GroundDraft)` (iterative 4-neighbor height clamping reading `LocomotionTable::shipped()` limits), `despeckle(&mut GroundDraft, min_region)` (4-neighbor class majority filter), `route(&mut GroundDraft)` (tracked A\* waypoint synthesis over the priced cost grid under `tracked_limit`), and `build_with_options(&GroundDraft, repair, force)` in [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs) and wired `--repair`, `--force`, and `ground-pack route <draft.json>` in [`crates/tacsim/src/bin/ground-pack.rs`](../../crates/tacsim/src/bin/ground-pack.rs) (D-30-9).

## WG.4 — Terrain & Arena Studio on `ground.draft.v1`

- Added `viewshed(&GroundDraft, ox, oy, size_cm)` (`0 = dead`, `1 = seen`, `2 = hull-down`, `3 = partial` over `ArenaTerrain::sight`) and `draft_from_world(&World, id)` in [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs), plus `ground-pack render <draft.json> --viewshed <x>,<y> [--size-cm <cm>]` in [`crates/tacsim/src/bin/ground-pack.rs`](../../crates/tacsim/src/bin/ground-pack.rs).
- Extended `MapQuery::query` in [`crates/tacsim/src/arena_bridge.rs`](../../crates/tacsim/src/arena_bridge.rs) to answer `arena.map` queries carrying `params.draft` with the requested `ground.draft.v1` document (`draft_from_world`), and wired `requestGroundDraft` / `groundDraftFrame` in [`web/src/lib/game.svelte.ts`](../../web/src/lib/game.svelte.ts).
- Updated [`web/src/lib/map-editor.ts`](../../web/src/lib/map-editor.ts) and [`web/src/components/MapEditorStudio.svelte`](../../web/src/components/MapEditorStudio.svelte):
  - Aligned `TERRAIN_PALETTE` and `CLASS_META` to the exact `mush_atlas::TerrainClass` codes (`0..=17`), words, and glyphs.
  - Replaced the 8-byte `Cells` codec with the 3-byte `[cols u16][rows u16] + [count u16][class u8]` RLE class codec (`encodeClassesBase64` / `decodeClassesBase64`), `le16` elevation codec (`encodeElevLe16Base64` / `decodeElevLe16Base64`), and `zigzag-leb128-delta` (`encodeElevZigzagDelta`).
  - Replaced `ArenaDef` export/import with `ground.draft.v1` (`exportGroundDraftJson` / `importGroundDraftJson`), added `256×256` grid preset and `± 150 m` height band (`DEFAULT_SLIDER_MIN_ELEV_CM` / `DEFAULT_SLIDER_MAX_ELEV_CM`), and removed the `@atlas/window` pre-fill (`D-14-28`).
  - Updated `editorToPictureFeeds` to emit real `arena.map` (`elev` as `zigzag-leb128-delta`, `surface: "tacsim.arena." + id`, `spots`, first-seen `legend`, `origin_cell: [0, 0]`) and real `arena.cockpit` pose keys (`x_cm`, `y_cm`, `z_cm`, `heading_mdeg`).
  - Added `computePassabilityOverlay`, `checkEditorMap`, `routeEditorMap`, `computeViewshed`, `relaxEditorSlopes`, and `despeckleEditorMap` over [`web/src/lib/fixtures/locomotion.json`](../../web/src/lib/fixtures/locomotion.json), and verified parity with `tacsim::ground` and `mush-pixel-wasm` in [`web/src/lib/map-editor.test.ts`](../../web/src/lib/map-editor.test.ts) and [`web/src/components/MapEditorStudio.test.ts`](../../web/src/components/MapEditorStudio.test.ts).
