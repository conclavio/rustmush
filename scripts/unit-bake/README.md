# unit-bake — 3D models to the unit pack (doc 22 D-22-10, cut U.6)

The bake matrix: for every shipped look, two views (`fp` at 3° down for the
first-person pictures, `iso` at the 2.5D's 28°) × eight facings × six poses
(still, a four-frame moving cycle, the wreck) × a size chain each (`fp` 96, 64,
48, 32, 24, 16, 12, 8 px; `iso` 96, 80, 64, 56, 48, 40, 32, 24 px), every size
reduced from one 384 px render of one named asset by one script, in
`crates/mush-pixel/looks/units.bin` (format 2: named entries keyed by look id,
each with its provenance).

**Never commit the asset packs, the Unity project or the raw renders.** The
licences let the models be used in this game, not redistributed (the owner's
ruling of 2026-10-29): only the configs, the scripts and the reduced pack ship,
and the pack names the asset pack, its version, the prefab, the clips and the
commits that made it, so anyone with their own licence can make it again.

## The one command

```bash
UNITY_PACKAGES=/c/Users/you/3d_assets scripts/unit-bake/bake.sh /c/scratch/unitybake
```

- The first argument is a scratch Unity project **outside the repo** (created
  when missing; Unity 6000.6.7f1 by default, `UNITY=...` for another).
- `UNITY_PACKAGES` is the folder each config's `package` path is relative to;
  a look's `.unitypackage` is imported once, when its `root` folder is not in
  the project yet.
- Name looks after the project to bake only those (`... unitybake biped drone`);
  with none, all six. `UNIT_PACK=/elsewhere/units.bin` writes a trial pack and
  leaves the tree's alone.
- Each look: `Bake.cs` renders it (`bake_out/<look>/`, 192 PNGs and
  `meta.json`), then the reducer (`crates/mush-pixel/examples/unit_reduce.rs`)
  puts its entry into the pack and prints the entry's size by chain, the
  pack's size, its digest and the matrix check, and writes
  `bake_out/preview_<look>.png` (every size's still frames, both views).

Then, in the tree:

1. Pin the printed digest in `crates/mush-pixel/src/bake.rs` (`PACK_DIGEST`,
   the matrix check's test: the pack changes only on purpose).
2. Render the line-ups:
   `cargo run -p mush-pixel --example unit_lineup -- docs/design/images/22-u6`.
3. `cargo test -p mush-pixel` (the matrix check: every look baked, both chains
   at every size, the marks inside every frame, each look's height and anchor
   agreed with its look file).

Commit first and bake from a clean tree: the provenance records the commit
`HEAD` names, with `-dirty` when the scripts or the reducer have changes.

## A look's config — `looks/<look>.json`

| Key | What |
|---|---|
| `look` | the look's id: its file's stem under `crates/mush-pixel/looks/` |
| `asset_pack`, `asset_version` | the pack's name as its store lists it, and its version (empty: the package file's SHA-256, which `bake.sh` passes) |
| `package` | the `.unitypackage`, relative to `UNITY_PACKAGES` |
| `root` | the folder the package imports into (`Assets/Mech_Constructor_Spiders/`) |
| `prefab`, `attach` | the base prefab, and parts placed in its mounts in order: `{prefab, mount, parent}` (parent `""` is the base, else an earlier part's prefab name); a missing mount fails the bake naming the mounts there are |
| `primitive` | a Unity built-in (`Sphere`, …) instead of a prefab — the `unknown` look's hump |
| `id_texture` | the colour-ID texture the kit's materials share (the Mech Constructor kits); without one, `material_parts` maps material-name substrings to parts (`paint`, `second`, `dark`, `accent`, `bright`) and `default_part` the rest |
| `still`, `moving`, `wreck` | clips `{file, name}`: the still at its first frame, the moving cycle at four quarters, the wreck at the fall's last frame (recentred over the origin); no `moving` clip bakes four still frames; no `wreck` clip tilts the still by `wreck_pose` (`tilt_deg`, `roll_deg`) |
| `muzzles` | `{locator, weapon}`: a transform path (`Weapon_Sniper_lvl3/Barrel_end`) and the weapon component id its shot leaves for |
| `size_cm` | the look's `[width, height]`: the model is scaled to the height, and its widest silhouette is measured (the matrix check holds it within half to twice the width: barrels stand past the sim's silhouette) |
| `yaw_offset_deg` | for a model whose front is not +Z |
| `id_classes` | optional: eight parts for the colour-ID bits `r·4 + g·2 + b` (the default is the Mech Constructor kits') |

The six configs name assemblies from the owner's Mech Constructor packs
(the light biped from *Light and Medium Robots* as the sprite-vs-voxel PoC
built it; the tank, the heavy, the scout buggy and the spider drone from
*Spiders and Tanks*; the `unknown` hump from a Unity primitive). They are a
first pick: change a config and bake that look again.

## What each piece does

- `unity/Editor/Bake.cs` — assembles the unit from its config, scales it to
  its look's height standing on the ground, and renders each view, facing and
  pose twice at 384 px orthographic: the colour-ID pass (`IdOut.shader` with
  the kit's texture, or `FlatId.shader` a part) and world normals
  (`NormalOut.shader`). The frame spans the unit's turning circle at any
  facing, so every frame of a view is to one scale (`span_cm` in `meta.json`);
  the anchor is the ground point under the unit's pivot, the muzzles the
  locators' points.
- `crates/mush-pixel/src/bake/reduce.rs` (run by `examples/unit_reduce.rs`) —
  the part (the colour-ID bits) lit by a light fixed to the viewer picks each
  render pixel's material: the main paint into the three team tones by light,
  secondary panels steel or team-dark, dark metal black, accents team-light,
  lamps and glass pale. Each size is reduced from the 384 px render as masses
  (a cell drawn at 30 % coverage, its most common material), its frame kept
  as a tight box, its anchor and muzzles scaled.

The sprite-vs-voxel PoC's `reduce.py` and voxel grids are gone (D-22-9 chose
sprites; the reducer is the Rust one, tested on a synthetic render in
`cargo test -p mush-pixel`).
