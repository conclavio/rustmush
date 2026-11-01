# `scripts/ship-bake/` — 3D Spaceship, Asteroid & Docking Wireframe Baker

Extracts 3D meshes (`Vertices` and `PolygonVertexIndex`) from external `MSGDI` space
`.unitypackage` archives (kept outside the repository per the licensing ruling and doc 30
`Q-30-9`) and bakes:

1. **8-Facing Shaded Sprite Sheets (`crates/mush-pixel/ships/<id>.png`)**:
   - `512×64` 8-bit indexed-colour PNGs (`8` yaw facings of `64×64` pixels at `0°, 45°, …, 315°`)
     with transparent background (`index 0`), Lambertian + rim-lit hull shading (`1..=239`),
     1 px hull silhouette outline (`240`), and stern thruster nozzle ports (`241`).
   - Encoded with a single stored (`level=0`) RFC 1951 `IDAT` block (`32,832` bytes) so each
     sheet is both a standard viewable PNG and directly indexable in $O(1)$ across native and
     `--no-default-features` WASM builds.
2. **Decimated 3D Feature-Edge Wireframes (`docking_wireframe_mm` in `index.json`)**:
   - Sharp crease and silhouette 3D edges extracted from the `.fbx` hull meshes for the 60 fps
     6-DOF Docking Camera (`crates/mush-pixel/src/docking.rs`).

## Usage

Run in isolated mode (`python -I`, Python 3 standard library only):

### 1. Bake all 8 canonical spaceship/asteroid sheets and wireframes

```sh
python -I scripts/ship-bake/bake.py \
  --unitypackage-dir "C:/Users/faeni/3d_assets" \
  --out-dir crates/mush-pixel/ships
```

### 2. Inspect `.fbx` meshes inside any `.unitypackage` archive

```sh
python -I scripts/ship-bake/bake.py \
  --list-assets "C:/Users/faeni/3d_assets/MSGDI/3D ModelsVehiclesSpace/Spaceship Freighters.unitypackage"
```

### 3. Run the built-in synthetic `.unitypackage` self-test

```sh
python -I scripts/ship-bake/bake.py --self-test
```
