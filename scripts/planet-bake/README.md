# `scripts/planet-bake/` — Solar System Equirectangular Planet Map Baker

Extracts high-resolution (`2048×1024` to `12288×6144`) 2:1 equirectangular planetary
surface maps from `Solar System Planet Creator.unitypackage` (kept outside the repository
per the licensing ruling and doc 30 `Q-30-9`) and bakes compact **`128×64` 8-bit
indexed-colour `.png` maps** into `crates/mush-pixel/planets/` alongside `index.json`
provenance metadata.

Per repository licensing rules:
- The external `.unitypackage` archive and its raw 4K–12K source PNGs are **never**
  copied or committed into the repository.
- Only the reduced `128×64` baked `.png` maps (~9 KB each) and `index.json` ship in
  `crates/mush-pixel/planets/`.
- Each baked `.png` uses an 8-bit indexed palette (`PLTE`) interpolated across the
  body's 6-step `mush-pixel` ramp and a stored (`level=0`) `IDAT` chunk so it is both a
  standard viewable colour PNG and directly sampleable in $O(1)$ by
  `mush_pixel::planet_shader` in both native and `--no-default-features` WASM builds.

## Usage

Run in isolated mode (`python -I`, Python 3 standard library only):

### 1. Bake all 12 solar-system planetary maps into `crates/mush-pixel/planets/`

```sh
python -I scripts/planet-bake/bake.py \
  --unitypackage-dir "C:/Users/faeni/3d_assets" \
  --out-dir crates/mush-pixel/planets
```

### 2. Inspect planetary texture maps inside a `.unitypackage` archive

```sh
python -I scripts/planet-bake/bake.py \
  --list-assets "C:/Users/faeni/3d_assets/galactica studio/3D ModelsEnvironmentsSci-Fi/Solar System Planet Creator.unitypackage"
```

### 3. Run the built-in synthetic `.unitypackage` self-test

```sh
python -I scripts/planet-bake/bake.py --self-test
```
