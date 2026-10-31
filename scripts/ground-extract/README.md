# `scripts/ground-extract/` — Unity Terrain & Splatmap Extractor

Extracts heightmaps and splatmaps from external `.unitypackage` archives (kept outside the
repository per the licensing ruling and doc 30 `Q-30-9`) and emits a `ground.draft.v1` JSON
document with full `provenance` (`source: "extractor"`, `inputs: [...]`).

Slope relaxation (`tacsim::ground::relax_slopes`) and speckle filtering
(`tacsim::ground::despeckle`) are **not** run in Python; `ground-pack build` runs them
automatically for `source: "extractor"` drafts before quadranting and signing the atlas and
arena packs.

## Usage

Run in isolated mode (`python -I`, Python 3 standard library only):

### 1. Reproduce the stitched `winter_valley` arena draft

```sh
python -I scripts/ground-extract/extract.py \
  --unitypackage-dir "C:/Users/faeni/3d_assets" \
  --out crates/tacsim/content/ground/winter_valley.draft.json

cargo run --package tacsim --bin ground-pack -- \
  build crates/tacsim/content/ground/winter_valley.draft.json \
  --out crates/tacsim/content
```

### 2. Inspect `TerrainData` assets inside any `.unitypackage` archive

```sh
python -I scripts/ground-extract/extract.py \
  --list-assets "C:/Users/faeni/3d_assets/MyEnvironment.unitypackage"
```

### 3. Unpack any single `.unitypackage` `TerrainData` asset into a `ground.draft.v1` JSON

```sh
python -I scripts/ground-extract/extract.py \
  --package "C:/Users/faeni/3d_assets/MyEnvironment.unitypackage" \
  --asset-suffix "Terrain.asset" \
  --id custom_basin \
  --label "Custom Basin" \
  --biome mars \
  --body mars \
  --cols 256 --rows 256 \
  --origin-m 45000,45000 \
  --raw-out-dir target/ground-raw \
  --out custom_basin.draft.json
```

### 4. Run the built-in synthetic `.unitypackage` self-test

```sh
python -I scripts/ground-extract/extract.py --self-test
```
