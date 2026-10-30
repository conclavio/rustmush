# World Generator v2 — Cut Notes

Per [`docs/design/30-world-generator-v2.md`](30-world-generator-v2.md) §11, each cut on the WG workstream records its landed scope and verification here.

## WG.0 — Design Document & Index

- Landed [`docs/design/30-world-generator-v2.md`](30-world-generator-v2.md) (Draft 0.2) and [`docs/design/22-unit-kit.md`](22-unit-kit.md) (Draft 0.5) via PRs #183 and #184.

## WG.1 — `tacsim::ground`, `ground-pack`, and the Two-Half Content Pin

- Added [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs):
  - `GroundDraft` (`ground.draft.v1`, D-30-3) with 3-byte `[cols u16][rows u16] + [count u16][class u8]` class RLE and `le16` elevation grid, validation (`Void` refusal, provenance input enforcement), and `generate` moved from `tests/support/ground.rs`.
  - `build(&GroundDraft)`: splits 64×64 / 128×128 / 256×256 drafts into 128×128 quadrant heightfield entries, formats the region and campaign-seed packs, computes canonical `DefinitionDigest` values, and enforces `arena_map_within_bound` (`MAP_MAX_BYTES`) before writing a byte (D-30-4, D-30-8).
  - `check(&GroundDraft)`: size-only report (`cols`, `rows`, `quadrants`, `map_bytes`, `map_max_bytes`) for WG.1 (extended with grade/passability in WG.3).
  - `from_packs(atlas_json, arena_json)`: inverse reconstruction (`pack → draft`) verified to round-trip `proving_ground` byte for byte.
- Added [`crates/tacsim/src/bin/ground-pack.rs`](../../crates/tacsim/src/bin/ground-pack.rs) CLI binary with `build`, `check`, `gen`, and `from-packs` subcommands.
- Checked in [`crates/tacsim/content/ground/proving_ground.draft.json`](../../crates/tacsim/content/ground/proving_ground.draft.json) and updated [`crates/tacsim/tests/shard_2/content.rs`](../../crates/tacsim/tests/shard_2/content.rs) to the two-half content pin (`the_shipped_draft_is_this_generators_output` and `every_shipped_ground_pack_is_the_writers_output_over_its_draft`).

## WG.2 — Picture Hygiene in `mush-pixel`

- Removed `Terrain::synthesize_relief` from [`crates/mush-pixel/src/heightfield.rs`](../../crates/mush-pixel/src/heightfield.rs) and the `procedural_wanted` / `all_flat` fallback from `terrain_from_feed` in [`crates/mush-pixel/src/cockpit.rs`](../../crates/mush-pixel/src/cockpit.rs): no `mush-pixel` path writes an `elev_cm` the `arena.map` feed did not carry (D-30-1, D-30-6).
- Tightened `Biome::from_name` in [`crates/mush-pixel/src/raycast.rs`](../../crates/mush-pixel/src/raycast.rs) and `map_from_feed` in [`crates/mush-pixel/src/cockpit.rs`](../../crates/mush-pixel/src/cockpit.rs) to an exact match on the four biome words (`temperate | mars | lunar | ice`), retiring `surface` substring and legend-word sniffing (D-30-5).
- Implemented D-30-10 (`rock` as a surface material, not an extruded `Wall` block): removed `"rock"`, `"cliff"`, and `"building"` from `Ground::from_legend_name`'s `Ground::Wall` arm in [`crates/mush-pixel/src/raycast.rs`](../../crates/mush-pixel/src/raycast.rs), and retired the `is_rock_spire` woods-as-spires branch on Mars/Lunar in [`crates/mush-pixel/src/heightfield.rs`](../../crates/mush-pixel/src/heightfield.rs) and [`crates/mush-pixel/src/iso25d.rs`](../../crates/mush-pixel/src/iso25d.rs).
- Added `a_flat_field_draws_flat_under_every_biome` and `a_rock_cell_at_elev_cm_0_blocks_no_sight_line_in_any_picture` to [`crates/mush-contract-tests/tests/pixel_relief.rs`](../../crates/mush-contract-tests/tests/pixel_relief.rs), and updated the 4-biome × 3-view distinctness test in [`crates/mush-pixel/src/iso25d.rs`](../../crates/mush-pixel/src/iso25d.rs) to supply explicit `elev` data.

## WG.3 — Sim Biome on `RegionDef` and `ground-pack check` / `route`

- Added `pub const BIOMES: [&str; 4] = ["temperate", "mars", "lunar", "ice"]` and `biome: Option<String>` to [`RegionDef`](../../crates/mush-atlas/src/def.rs) and [`RegionRec`](../../crates/mush-atlas/src/record.rs), validated in `RegionRec::validate` and `mush_atlas::validate` (D-30-5).
- Added `biome: SmolStr` (`biome()`, `with_biome(...)`) to [`ArenaTerrain`](../../crates/tacsim/src/arena/state.rs), populated from `RegionRec::biome_str()` in `ArenaDef::from_pack`, and emitted as `"biome": terrain.biome()` on the `arena.map` frame in [`crates/tacsim/src/match_/cockpit/frames.rs`](../../crates/tacsim/src/match_/cockpit/frames.rs).
- Removed `ArenaDef.biome`, `ArenaDef.procedural_relief`, `ArenaDef::biome_variant`, `SIN64`, and `apply_biome_and_relief` from [`crates/tacsim/src/definitions/arena.rs`](../../crates/tacsim/src/definitions/arena.rs) (D-30-1, D-30-5).
- Extended `CheckReport` and `check(&GroundDraft)` in [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs) with the full §5 grade and passability report: tracked / wheeled / legged passability shares (`locomotion.json` rules), zone-to-zone and key-spot reachability, waypoint graph reachability, max cell and road grade (`%`), entry-zone step check, and failure list.
- Added `relax_slopes(&mut GroundDraft)` (iterative 4-neighbor height clamping reading `LocomotionTable::shipped()` limits), `despeckle(&mut GroundDraft, min_region)` (4-neighbor class majority filter), `route(&mut GroundDraft)` (tracked A\* waypoint synthesis over the priced cost grid under `tracked_limit`), and `build_with_options(&GroundDraft, repair, force)` in [`crates/tacsim/src/ground.rs`](../../crates/tacsim/src/ground.rs) and wired `--repair`, `--force`, and `ground-pack route <draft.json>` in [`crates/tacsim/src/bin/ground-pack.rs`](../../crates/tacsim/src/bin/ground-pack.rs) (D-30-9).
