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
