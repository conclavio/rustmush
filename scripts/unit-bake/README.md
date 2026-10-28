# unit-bake — 3D models to sprite sheets (doc 22 D-22-9)

The pipeline the 2026-10-28 sprite-vs-voxel PoC was built with: a licensed Unity
asset (a `.unitypackage` the owner bought) is assembled, posed and rendered in
Unity's batch mode, and the renders are reduced to doc 22 sheets — eight facings,
six poses (still, a four-frame moving cycle, the wreck), the six materials.

**Never commit the source assets, the Unity project, or the raw renders.** The
licence lets the models be used in this game, not redistributed as assets; only
the reduced sheets (and code) go in the repo. Keep the Unity project outside the
tree (the PoC used a scratch directory).

## Steps

```bash
UNITY="/c/Program Files/Unity/Hub/Editor/6000.6.7f1/Editor/Unity.exe"
PROJ=/path/outside/the/repo/unitybake

# 1. A built-in-pipeline project, and the package imported into it.
"$UNITY" -batchmode -quit -createProject "$PROJ"
"$UNITY" -batchmode -quit -projectPath "$PROJ" -importPackage "/path/to/Pack.unitypackage"

# 2. The bake script and its two shaders into the project, then the bake:
#    <PROJ>/bake_out gets {facing}_{pose}_{id,n}.png, meta.json, vox{48,96}_{pose}.bin
cp -r scripts/unit-bake/unity/* "$PROJ/Assets/"
"$UNITY" -batchmode -quit -projectPath "$PROJ" -executeMethod Bake.Run -logFile bake.log

# 3. Reduce to sheets (run from a directory of your own; needs numpy + Pillow).
python scripts/unit-bake/reduce.py "$PROJ/bake_out" out/
```

## What each piece does

- `unity/Editor/Bake.cs` — assembles one mech from the Mech Constructor kit
  (legs, cockpit, weapons into the kit's `Mount_*` sockets; the parts, clips and
  colour-ID texture are named at the top — **PoC-specific**, generalise per
  pack), poses it from the walk and fall clips, and renders each facing twice at
  384 px orthographic, 8 degrees down: the colour-ID texture (`IdOut.shader`)
  and world normals (`NormalOut.shader`). The wreck is recentred on the ground.
  Anchor and muzzle points (`Barrel_end` locators) are projected into
  `meta.json`. It also writes solid voxel grids (the PoC's comparison; not
  needed for sheets).
- `reduce.py` — colour-ID class plus a fixed viewer-relative light picks the
  material per render pixel (main paint into the three team tones by light,
  secondary panels steel or team-dark, dark metal black, lights and glass
  pale); a block reduce to the frame size keeps a cell at 30 % coverage and
  takes its most common material. Writes `sheet{48,96}.json` in the PoC's
  eight-facing layout and a preview PNG.

The output is not yet the shipped look format: that takes `size_cm`, the unit's
weapon ids for muzzles, and the format's eight facings (doc 22 D-22-9, owed).
