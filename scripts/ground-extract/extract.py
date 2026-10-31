#!/usr/bin/env python3
"""Extract and stitch Unity terrain heightmaps and splatmaps into a `ground.draft.v1` JSON.

Run in isolated mode (`python -I`, stdlib only):

1. Reproduce the stitched `winter_valley` arena draft from external `.unitypackage` archives:

    python -I scripts/ground-extract/extract.py \\
        --unitypackage-dir "C:/Users/faeni/3d_assets" \\
        --out crates/tacsim/content/ground/winter_valley.draft.json

2. Inspect all Unity `TerrainData` assets inside a `.unitypackage` archive:

    python -I scripts/ground-extract/extract.py \\
        --list-assets "C:/Users/faeni/3d_assets/Package.unitypackage"

3. Unpack any single `.unitypackage` `TerrainData` asset into a `ground.draft.v1` JSON
   (plus optional raw `.le16` and `.rle` files):

    python -I scripts/ground-extract/extract.py \\
        --package "C:/Users/faeni/3d_assets/Package.unitypackage" \\
        --asset-suffix "Terrain.asset" \\
        --id custom_basin --label "Custom Basin" --biome mars --body mars \\
        --out custom_basin.draft.json

4. Run the built-in self-test (no external `.unitypackage` files required):

    python -I scripts/ground-extract/extract.py --self-test

Per doc 30 §9 and Q-30-9:
- Reads external `.unitypackage` archives (or pre-extracted `.asset` files) outside the repo.
- Emits a `ground.draft.v1` JSON with `provenance.source = "extractor"` and `provenance.inputs`
  naming the upstream Unity asset packs and versions.
- Does NOT run slope relaxation or speckle filtering in Python; `ground-pack build` runs the
  canonical Rust repair passes (`tacsim::ground::relax_slopes` and `tacsim::ground::despeckle`)
  and synthesizes the §5 check report.
"""

import argparse
import base64
import io
import json
import math
import os
import struct
import tarfile
import tempfile
from pathlib import Path

DRAFT_SCHEMA = "ground.draft.v1"
SIDE = 256
CELL_CM = 1000
ORIGIN_M = [30000, 30000]

RIVER = (62, 66)
FORD = (92, 102)
ROAD_EW = (96, 98)
ROAD_NS = (128, 130)

BUILDINGS = [
    (150, 170, 36, 52),
    (96, 116, 186, 202),
    (200, 216, 140, 156),
]

VALLEY_OUTPOSTS = [
    (36, 42, 86, 91),
    (82, 88, 103, 108),
    (172, 178, 86, 91),
    (220, 226, 103, 108),
]

BLUFF_TOP = (141, 147, 105, 118)
BLUFF_TOP_CM = 450
BLUFF_RAMP = [
    ((141, 146, 119, 119), 270),
    ((141, 146, 120, 120), 40),
]
BLUFF_KEEP = (138, 150, 102, 123)

CLASS_PAVED = 1
CLASS_GROUND = 2
CLASS_GRASS = 3
CLASS_WATER = 4
CLASS_FLOOR = 5
CLASS_RUBBLE = 6
CLASS_ROAD = 7
CLASS_SAND = 8
CLASS_ROCK = 9
CLASS_WALL = 10
CLASS_DOOR = 11
CLASS_PLATFORM = 12
CLASS_WOODS_LIGHT = 13
CLASS_WOODS_HEAVY = 14
CLASS_WATER_DEEP = 15
CLASS_ICE = 16
CLASS_REGOLITH = 17

UPSTREAM_INPUTS = [
    "Manufactura K4 - Winter Pack v1.0 (Demo valley Terrain.asset, 2049x2049)",
    "MSGDI - Arcane Forest Environment v1.0 (TerrainForest.asset, 513x513)",
    "MSGDI - Wasteland Environment v1.0 (TerainScene2.asset, 513x513)",
]

PACKAGE_SPECS = {
    "Demo valley Terrain.asset": (
        "Manufactura K4/3D ModelsEnvironments/Winter Pack.unitypackage",
        "Demo valley Terrain.asset",
    ),
    "TerrainForest.asset": (
        "MSGDI/3D ModelsEnvironmentsLandscapes/Arcane Forest Environment.unitypackage",
        "TerrainForest.asset",
    ),
    "TerainScene2.asset": (
        "MSGDI/3D ModelsEnvironmentsLandscapes/Wasteland Environment.unitypackage",
        "TerainScene2.asset",
    ),
}


def within(x, y, box):
    return box[0] <= x <= box[1] and box[2] <= y <= box[3]


def box_distance(x, y, box):
    dx = max(box[0] - x, 0, x - box[1])
    dy = max(box[2] - y, 0, y - box[3])
    return max(dx, dy)


def smoothstep(t):
    t = max(0.0, min(1.0, t))
    return t * t * (3.0 - 2.0 * t)


def extract_asset_from_unitypackage(pkg_path, target_suffix):
    """Read `<guid>/pathname` and `<guid>/asset` from a `.unitypackage` tar.gz archive in memory."""
    pathnames = {}
    assets = {}
    with tarfile.open(pkg_path, "r:gz") as tf:
        for member in tf.getmembers():
            if not member.isfile():
                continue
            parts = member.name.strip("./").split("/")
            if len(parts) != 2:
                continue
            guid, leaf = parts
            if leaf == "pathname":
                f = tf.extractfile(member)
                if f is not None:
                    raw_path = f.read().decode("utf-8", errors="replace").splitlines()[0].strip()
                    pathnames[guid] = raw_path
            elif leaf == "asset":
                assets[guid] = member

        for guid, internal_path in pathnames.items():
            if internal_path.endswith(target_suffix) and guid in assets:
                f = tf.extractfile(assets[guid])
                if f is not None:
                    return f.read()
    raise FileNotFoundError(f"Could not find `{target_suffix}` inside `{pkg_path}`")


def load_asset_bytes(asset_name, unitypackage_dir, extracted_dir):
    if extracted_dir:
        candidate = Path(extracted_dir) / asset_name
        if candidate.is_file():
            return candidate.read_bytes()
    if unitypackage_dir:
        rel_pkg, suffix = PACKAGE_SPECS[asset_name]
        pkg_path = Path(unitypackage_dir) / rel_pkg
        if pkg_path.is_file():
            return extract_asset_from_unitypackage(pkg_path, suffix)
    raise FileNotFoundError(
        f"Could not load `{asset_name}` from --extracted-dir={extracted_dir!r} "
        f"or --unitypackage-dir={unitypackage_dir!r}"
    )


def parse_unity_heightmap(data, res):
    count = res * res
    needle = struct.pack("<I", count)
    pos = data.rfind(needle, 0, len(data) - count * 2)
    if pos < 0:
        raise ValueError(f"Could not find binary heightmap of {count} samples")
    raw = data[pos + 4 : pos + 4 + count * 2]
    return struct.unpack(f"<{count}H", raw), res


def detect_unity_heightmap(data):
    """Auto-detect a Unity TerrainData heightmap (`(2^k + 1) x (2^k + 1)` u16 samples)."""
    for res in (4097, 2049, 1025, 513, 257, 129, 65, 33):
        count = res * res
        if len(data) < count * 2 + 4:
            continue
        needle = struct.pack("<I", count)
        pos = data.rfind(needle, 0, len(data) - count * 2 + 1)
        if pos >= 0:
            raw = data[pos + 4 : pos + 4 + count * 2]
            return struct.unpack(f"<{count}H", raw), res
    raise ValueError("Could not auto-detect Unity heightmap resolution in asset bytes")


def parse_splat_rgba(data):
    pos = data.find(b"SplatAlpha")
    if pos < 0:
        raise ValueError("Could not find SplatAlpha in Unity TerrainData asset")
    window = data[pos : pos + 200]
    for sz, w in (
        (1398100, 512),
        (5592404, 1024),
        (22369620, 2048),
        (262144, 256),
        (65536, 128),
        (16384, 64),
    ):
        needle = struct.pack("<I", sz)
        p1 = window.find(needle)
        if p1 != -1:
            p2 = window.find(needle, p1 + 4)
            if p2 != -1:
                pix_start = pos + p2 + 4
                if pix_start + w * w * 4 <= len(data):
                    return data[pix_start : pix_start + w * w * 4], w, w
    raise ValueError("Could not locate SplatAlpha pixel buffer")


def list_unitypackage_terrain_assets(pkg_path):
    """Return a list of `(internal_path, hm_res, splat_dims)` for TerrainData assets in `pkg_path`."""
    pathnames = {}
    assets = {}
    with tarfile.open(pkg_path, "r:gz") as tf:
        for member in tf.getmembers():
            if not member.isfile():
                continue
            parts = member.name.strip("./").split("/")
            if len(parts) != 2:
                continue
            guid, leaf = parts
            if leaf == "pathname":
                f = tf.extractfile(member)
                if f is not None:
                    raw_path = f.read().decode("utf-8", errors="replace").splitlines()[0].strip()
                    pathnames[guid] = raw_path
            elif leaf == "asset":
                assets[guid] = member

        found = []
        for guid, internal_path in sorted(pathnames.items(), key=lambda kv: kv[1]):
            if not internal_path.lower().endswith(".asset") or guid not in assets:
                continue
            f = tf.extractfile(assets[guid])
            if f is None:
                continue
            blob = f.read()
            try:
                _, hm_res = detect_unity_heightmap(blob)
            except ValueError:
                continue
            try:
                _, sw, sh = parse_splat_rgba(blob)
                splat_info = f"{sw}x{sh}"
            except ValueError:
                splat_info = "none"
            found.append((internal_path, hm_res, splat_info))
        return found


def sample_bilinear(grid, res, u, v):
    u = max(0.0, min(1.0, u))
    v = max(0.0, min(1.0, v))
    fx = u * (res - 1)
    fy = v * (res - 1)
    x0 = int(fx)
    y0 = int(fy)
    x1 = min(x0 + 1, res - 1)
    y1 = min(y0 + 1, res - 1)
    tx = fx - x0
    ty = fy - y0
    v00 = grid[y0 * res + x0]
    v10 = grid[y0 * res + x1]
    v01 = grid[y1 * res + x0]
    v11 = grid[y1 * res + x1]
    return (v00 * (1.0 - tx) + v10 * tx) * (1.0 - ty) + (v01 * (1.0 - tx) + v11 * tx) * ty


def sample_splat_rgba(data, width, height, u, v):
    u = max(0.0, min(1.0, u))
    v = max(0.0, min(1.0, v))
    ix = min(int(u * (width - 1)), width - 1)
    iy = min(int(v * (height - 1)), height - 1)
    off = (iy * width + ix) * 4
    return data[off], data[off + 1], data[off + 2], data[off + 3]


def encode_classes_rle(cols, rows, flat_classes):
    buf = bytearray()
    buf.extend(struct.pack("<HH", cols, rows))
    prev_code = None
    run_len = 0
    for code in flat_classes:
        c = int(code)
        if prev_code == c and run_len < 65535:
            run_len += 1
        else:
            if prev_code is not None:
                buf.extend(struct.pack("<HB", run_len, prev_code))
            prev_code = c
            run_len = 1
    if prev_code is not None:
        buf.extend(struct.pack("<HB", run_len, prev_code))
    return base64.b64encode(buf).decode("ascii")


def encode_elev_le16(flat_elev):
    buf = bytearray(len(flat_elev) * 2)
    for i, val in enumerate(flat_elev):
        clamped = max(-32768, min(32767, int(val)))
        struct.pack_into("<h", buf, i * 2, clamped)
    return base64.b64encode(buf).decode("ascii")


def stitch_winter_valley(unitypackage_dir, extracted_dir):
    valley_bytes = load_asset_bytes("Demo valley Terrain.asset", unitypackage_dir, extracted_dir)
    forest_bytes = load_asset_bytes("TerrainForest.asset", unitypackage_dir, extracted_dir)
    waste_bytes = load_asset_bytes("TerainScene2.asset", unitypackage_dir, extracted_dir)

    valley_hm, valley_res = parse_unity_heightmap(valley_bytes, 2049)
    valley_splat, vs_w, vs_h = parse_splat_rgba(valley_bytes)

    forest_hm, forest_res = parse_unity_heightmap(forest_bytes, 513)
    forest_splat, fs_w, fs_h = parse_splat_rgba(forest_bytes)

    waste_hm, waste_res = parse_unity_heightmap(waste_bytes, 513)
    waste_splat, ws_w, ws_h = parse_splat_rgba(waste_bytes)

    def sample_aligned_valley(x, y):
        t = x / 255.0
        s = (y - 97.0) / 160.0
        u_center = 0.10 + 0.80 * t
        v_center = 0.57 - 0.19 * t + 0.03 * math.sin(t * math.pi * 2.0)
        u = u_center + s * 0.22
        v = v_center + s * 0.58
        h_raw = sample_bilinear(valley_hm, valley_res, u, v) / 32768.0
        h_center = sample_bilinear(valley_hm, valley_res, u_center, v_center) / 32768.0
        splat = sample_splat_rgba(valley_splat, vs_w, vs_h, u, v)
        return h_raw, h_center, splat

    def is_spur_road(x, y):
        # Paved/road spurs connecting North Barn (106, 194) and East Compound (208, 148)
        # to the main road cross so wheeled vehicles (<= 25% grade) can reach every key spot.
        if 106 <= x <= 128 and 193 <= y <= 195:
            return True
        if 199 <= x <= 209 and 98 <= y <= 148 and (x in (200, 208) or y in (138, 139)):
            return True
        if 130 <= x <= 144 and y == 121:
            return True
        return False

    def is_protected_corridor(x, y):
        if ROAD_EW[0] - 2 <= y <= ROAD_EW[1] + 2:
            return True
        if ROAD_NS[0] - 2 <= x <= ROAD_NS[1] + 2:
            return True
        if is_spur_road(x, y):
            return True
        if x <= 18 or x >= 237:
            return True
        if 196 <= x <= 212 and 95 <= y <= 158:
            return True
        if 94 <= x <= 132 and 184 <= y <= 204:
            return True
        if 122 <= x <= 136 and 90 <= y <= 104:
            return True
        if within(x, y, BLUFF_KEEP):
            return True
        if math.hypot(x - 88.0, y - 36.0) < 27.0:
            return True
        return False

    elev_cm = [[0.0] * SIDE for _ in range(SIDE)]
    class_grid = [[CLASS_GRASS] * SIDE for _ in range(SIDE)]

    for y in range(SIDE):
        for x in range(SIDE):
            vh, vc, _ = sample_aligned_valley(x, y)
            fu = (x / 255.0) * 0.85 + 0.07
            fv = (y / 255.0) * 0.85 + 0.07
            fh = sample_bilinear(forest_hm, forest_res, fu, fv) / 32768.0

            wu = ((255 - x) / 255.0) * 0.85 + 0.07
            wv = (y / 255.0) * 0.85 + 0.07
            wh = sample_bilinear(waste_hm, waste_res, wu, wv) / 32768.0

            valley_wall_m = max(0.0, (vh - vc) * 95.0)
            valley_floor_m = (vc - 0.115) * 55.0
            forest_m = (fh - 0.18) * 28.0
            waste_m = (wh - 0.25) * 26.0

            dist_ew = abs(y - 97.0)
            dist_ns = abs(x - 129.0)
            dist_road = min(dist_ew, dist_ns)

            # Blend down near the east/west entry zones (x <= 15 and x >= 240) so entry zones stay flat
            edge_dist = min(max(0.0, x - 16.0), max(0.0, 239.0 - x))
            edge_w = smoothstep(edge_dist / 14.0)

            canyon_w = smoothstep((dist_ew - 3.5) / 10.0) * smoothstep((dist_ns - 3.5) / 9.0) * edge_w
            foothill_band = (
                smoothstep((dist_ew - 4.0) / 6.0)
                * (1.0 - 0.4 * smoothstep((dist_ew - 22.0) / 15.0))
                * edge_w
            )
            foothill_m = foothill_band * (
                7.5 * max(0.0, fh - 0.12)
                + 6.5 * max(0.0, wh - 0.18)
                + 4.5 * math.sin(x / 7.5 + y / 9.0) * math.cos(x / 11.0 - y / 6.5)
            )

            combined_m = (
                edge_w * valley_floor_m * 0.55
                + canyon_w * (valley_wall_m + 0.55 * forest_m + 0.45 * waste_m)
                + foothill_m
            )

            river_dist = abs(x - 64.0)
            if river_dist < 14.0:
                gorge_t = 1.0 - smoothstep(river_dist / 14.0)
                ford_keep = 1.0 - 0.85 * (1.0 - smoothstep(max(0.0, abs(y - 97.0) - 4.0) / 8.0))
                combined_m -= gorge_t * 5.0 * ford_keep * edge_w

            sig_r = math.hypot(x - 185.0, y - 215.0)
            if sig_r < 45.0:
                combined_m += 42.0 * math.exp(-(sig_r * sig_r) / (2.0 * 18.0 * 18.0)) * edge_w

            bowl_r = math.hypot(x - 88.0, y - 36.0)
            if bowl_r < 26.0:
                if bowl_r < 15.0:
                    combined_m += (-9.0 + 18.0 * ((bowl_r / 15.0) ** 2)) * edge_w
                else:
                    combined_m += 9.0 * math.exp(-(((bowl_r - 15.0) / 6.5) ** 2)) * edge_w

            elev_cm[y][x] = combined_m * 100.0

            placed = False
            for b in BUILDINGS:
                if within(x, y, b):
                    door_x = (b[0] + b[1]) // 2
                    if (y == b[2] and x == door_x) or (b == (96, 116, 186, 202) and x == b[1] and y == 194):
                        class_grid[y][x] = CLASS_DOOR
                    elif x in (b[0], b[1]) or y in (b[2], b[3]):
                        class_grid[y][x] = CLASS_WALL
                    else:
                        class_grid[y][x] = CLASS_FLOOR
                    placed = True
                    break
            if placed:
                continue

            for b in VALLEY_OUTPOSTS:
                if within(x, y, b):
                    door_x = (b[0] + b[1]) // 2
                    door_y = b[2] if b[2] > 97 else b[3]
                    if y == door_y and x == door_x:
                        class_grid[y][x] = CLASS_DOOR
                    elif x in (b[0], b[1]) or y in (b[2], b[3]):
                        class_grid[y][x] = CLASS_WALL
                    else:
                        class_grid[y][x] = CLASS_FLOOR
                    placed = True
                    break
            if placed:
                continue

            if RIVER[0] <= x <= RIVER[1]:
                if FORD[0] <= y <= FORD[1]:
                    class_grid[y][x] = CLASS_WATER
                else:
                    class_grid[y][x] = CLASS_WATER_DEEP
                continue

            if (ROAD_EW[0] <= y <= ROAD_EW[1]) or (ROAD_NS[0] <= x <= ROAD_NS[1]) or is_spur_road(x, y):
                class_grid[y][x] = CLASS_ROAD
                continue

            if within(x, y, (30, 48, 165, 185)) or within(x, y, (175, 193, 50, 70)):
                class_grid[y][x] = CLASS_WOODS_HEAVY
                continue
            if within(x, y, (20, 58, 150, 200)) or within(x, y, (160, 210, 40, 80)):
                class_grid[y][x] = CLASS_WOODS_LIGHT
                continue

            bx = (x // 4) * 4 + 2
            by = (y // 2) * 2 + 1
            _, _, vsplat_b = sample_aligned_valley(bx, by)
            fsplat_b = sample_splat_rgba(
                forest_splat, fs_w, fs_h, (bx / 255.0) * 0.85 + 0.07, (by / 255.0) * 0.85 + 0.07
            )
            wsplat_b = sample_splat_rgba(
                waste_splat, ws_w, ws_h, ((255 - bx) / 255.0) * 0.85 + 0.07, (by / 255.0) * 0.85 + 0.07
            )

            vr, vg, vb, _ = vsplat_b
            fr, fg, fb, _ = fsplat_b
            wr, _, _, _ = wsplat_b

            prot = is_protected_corridor(x, y)
            forest_score = int(vg) + int(fg) // 2
            rock_score = int(vr) + int(wr) // 2
            stream_score = int(vb) + int(fb) // 2
            grove_mask = math.sin(bx * 0.14 + by * 0.11) + 0.65 * math.cos(bx * 0.09 - by * 0.17)

            if not prot and forest_score > 185 and grove_mask > 0.35 and dist_road >= 5.0:
                if forest_score > 230 and grove_mask > 0.80 and dist_road >= 8.0:
                    class_grid[y][x] = CLASS_WOODS_HEAVY
                else:
                    class_grid[y][x] = CLASS_WOODS_LIGHT
            elif (
                not prot
                and 4.0 <= dist_ew <= 10.0
                and dist_ns >= 4.0
                and forest_score > 145
                and grove_mask > 0.45
            ):
                class_grid[y][x] = CLASS_WOODS_LIGHT
            elif not prot and rock_score > 175 and dist_road >= 6.0:
                if rock_score > 225:
                    class_grid[y][x] = CLASS_RUBBLE
                else:
                    class_grid[y][x] = CLASS_GROUND
            elif dist_ew >= 3.0 and (rock_score > 145 or (combined_m > 18.0 and grove_mask < -0.15)):
                class_grid[y][x] = CLASS_GROUND
            elif not prot and stream_score > 195 and dist_ew >= 14.0 and combined_m < 4.0:
                class_grid[y][x] = CLASS_WATER
            else:
                class_grid[y][x] = CLASS_GRASS

    # Level building & outpost footprints onto their pad centres
    for b in BUILDINGS + VALLEY_OUTPOSTS:
        cx = (b[0] + b[1]) // 2
        cy = (b[2] + b[3]) // 2
        pad_cm = elev_cm[cy][cx]
        for y in range(max(0, b[2] - 4), min(SIDE, b[3] + 5)):
            for x in range(max(0, b[0] - 4), min(SIDE, b[1] + 5)):
                d = box_distance(x, y, (b[0] - 1, b[1] + 1, b[2] - 1, b[3] + 1))
                t = smoothstep(d / 4.0)
                elev_cm[y][x] = pad_cm + (elev_cm[y][x] - pad_cm) * t

    # Stamp The Bluff and its north ramp inside the BLUFF_KEEP apron
    for y in range(BLUFF_KEEP[2], BLUFF_KEEP[3] + 1):
        for x in range(BLUFF_KEEP[0], BLUFF_KEEP[1] + 1):
            elev_cm[y][x] = 0.0
            if not is_spur_road(x, y):
                class_grid[y][x] = CLASS_GRASS
    for y in range(BLUFF_TOP[2], BLUFF_TOP[3] + 1):
        for x in range(BLUFF_TOP[0], BLUFF_TOP[1] + 1):
            elev_cm[y][x] = float(BLUFF_TOP_CM)
    for box, ramp_cm in BLUFF_RAMP:
        for y in range(box[2], box[3] + 1):
            for x in range(box[0], box[1] + 1):
                elev_cm[y][x] = float(ramp_cm)

    # Quantize to 50 cm steps (no slope relaxation or despeckling here; `ground-pack build` runs them)
    flat_elev = []
    flat_classes = []
    for y in range(SIDE):
        for x in range(SIDE):
            if x <= 15 or x >= 240:
                q_cm = 0
            else:
                q_cm = int(round(elev_cm[y][x] / 50.0) * 50)
            flat_elev.append(q_cm)
            flat_classes.append(class_grid[y][x])

    return flat_classes, flat_elev


def build_draft_dict(flat_classes, flat_elev):
    nodes = [
        {"id": "ew_008", "x": 8, "y": 97},
        {"id": "ew_032", "x": 32, "y": 97},
        {"id": "ew_056", "x": 56, "y": 97},
        {"id": "ew_064", "x": 64, "y": 97},
        {"id": "ew_080", "x": 80, "y": 97},
        {"id": "ew_104", "x": 104, "y": 97},
        {"id": "xroads", "x": 129, "y": 97},
        {"id": "ew_152", "x": 152, "y": 97},
        {"id": "ew_176", "x": 176, "y": 97},
        {"id": "ew_200", "x": 200, "y": 97},
        {"id": "ew_224", "x": 224, "y": 97},
        {"id": "ew_248", "x": 248, "y": 97},
        {"id": "ns_008", "x": 129, "y": 8},
        {"id": "ns_032", "x": 129, "y": 32},
        {"id": "ns_056", "x": 129, "y": 56},
        {"id": "ns_080", "x": 129, "y": 80},
        {"id": "ns_120", "x": 129, "y": 120},
        {"id": "ns_144", "x": 129, "y": 144},
        {"id": "ns_168", "x": 129, "y": 168},
        {"id": "ns_192", "x": 129, "y": 192},
        {"id": "ns_216", "x": 129, "y": 216},
        {"id": "ns_248", "x": 129, "y": 248},
        {"id": "barn", "x": 106, "y": 194},
        {"id": "compound", "x": 208, "y": 148},
        {"id": "muster_red", "x": 6, "y": 105},
        {"id": "muster_blue", "x": 249, "y": 105},
    ]
    edges = [
        ["ew_008", "ew_032"],
        ["ew_032", "ew_056"],
        ["ew_056", "ew_064"],
        ["ew_064", "ew_080"],
        ["ew_080", "ew_104"],
        ["ew_104", "xroads"],
        ["xroads", "ew_152"],
        ["ew_152", "ew_176"],
        ["ew_176", "ew_200"],
        ["ew_200", "ew_224"],
        ["ew_224", "ew_248"],
        ["ns_008", "ns_032"],
        ["ns_032", "ns_056"],
        ["ns_056", "ns_080"],
        ["ns_080", "xroads"],
        ["xroads", "ns_120"],
        ["ns_120", "ns_144"],
        ["ns_144", "ns_168"],
        ["ns_168", "ns_192"],
        ["ns_192", "ns_216"],
        ["ns_216", "ns_248"],
        ["ns_192", "barn"],
        ["ew_200", "compound"],
        ["ew_008", "muster_red"],
        ["ew_248", "muster_blue"],
    ]
    unit = "tacsim.unit.line_tank"
    tanks = [
        {"id": "red_1", "name": "Red Tank 1", "team": "red", "x": 6, "y": 97, "heading_mdeg": 90000, "unit": unit},
        {"id": "red_2", "name": "Red Tank 2", "team": "red", "x": 6, "y": 95, "heading_mdeg": 90000, "unit": unit},
        {"id": "red_3", "name": "Red Tank 3", "team": "red", "x": 6, "y": 99, "heading_mdeg": 90000, "unit": unit},
        {"id": "red_4", "name": "Red Tank 4", "team": "red", "x": 6, "y": 93, "heading_mdeg": 90000, "unit": unit},
        {"id": "blue_1", "name": "Blue Tank 1", "team": "blue", "x": 249, "y": 97, "heading_mdeg": 270000, "unit": unit},
        {"id": "blue_2", "name": "Blue Tank 2", "team": "blue", "x": 249, "y": 95, "heading_mdeg": 270000, "unit": unit},
        {"id": "blue_3", "name": "Blue Tank 3", "team": "blue", "x": 249, "y": 99, "heading_mdeg": 270000, "unit": unit},
        {"id": "blue_4", "name": "Blue Tank 4", "team": "blue", "x": 249, "y": 93, "heading_mdeg": 270000, "unit": unit},
        {"id": "red_5", "name": "Red Tank 5", "team": "red", "x": 6, "y": 101, "heading_mdeg": 90000, "unit": unit},
        {"id": "red_6", "name": "Red Tank 6", "team": "red", "x": 4, "y": 94, "heading_mdeg": 90000, "unit": unit},
        {"id": "blue_5", "name": "Blue Tank 5", "team": "blue", "x": 249, "y": 101, "heading_mdeg": 270000, "unit": unit},
        {"id": "blue_6", "name": "Blue Tank 6", "team": "blue", "x": 251, "y": 94, "heading_mdeg": 270000, "unit": unit},
    ]
    return {
        "schema": DRAFT_SCHEMA,
        "id": "winter_valley",
        "label": "Winter Valley",
        "biome": "temperate",
        "body": "luna",
        "origin_m": ORIGIN_M,
        "cell_cm": CELL_CM,
        "cols": SIDE,
        "rows": SIDE,
        "classes": encode_classes_rle(SIDE, SIDE, flat_classes),
        "elev": encode_elev_le16(flat_elev),
        "entry_zones": [
            {"team": "red", "x0": 0, "y0": 0, "x1": 15, "y1": 255},
            {"team": "blue", "x0": 240, "y0": 0, "x1": 255, "y1": 255},
        ],
        "key_spots": [
            {"name": "The Ford", "x": 64, "y": 97},
            {"name": "The Crossroads", "x": 129, "y": 97},
            {"name": "North Barn", "x": 106, "y": 194},
            {"name": "East Compound", "x": 208, "y": 148},
            {"name": "The Bluff", "x": 144, "y": 111},
        ],
        "waypoints": {
            "nodes": nodes,
            "edges": edges,
        },
        "modes": [
            {"mode": "hold", "spot": "The Crossroads", "radius_cells": 6, "hold_ms": 120000},
        ],
        "tanks": tanks,
        "provenance": {
            "source": "extractor",
            "version": "1.0",
            "seed": 2350,
            "inputs": UPSTREAM_INPUTS,
        },
    }


def splat_to_biome_class(biome, r, g, b, a, norm_h):
    """Map Unity SplatAlpha RGBA weights + normalized height to a §6 biome terrain class."""
    if biome == "mars":
        if r >= 180 and r >= g and r >= b:
            return CLASS_ROCK
        if b >= 170:
            return CLASS_RUBBLE
        if g >= 135 or norm_h < 0.35:
            return CLASS_SAND
        return CLASS_GROUND
    if biome == "lunar":
        if r >= 185:
            return CLASS_ROCK
        if b >= 165:
            return CLASS_RUBBLE
        if g >= 145:
            return CLASS_GROUND
        return CLASS_REGOLITH
    if biome == "ice":
        if b >= 195 and norm_h < 0.25:
            return CLASS_WATER_DEEP
        if r >= 180:
            return CLASS_ROCK
        if g >= 150:
            return CLASS_GROUND
        return CLASS_ICE
    # temperate default
    if b >= 195 and norm_h < 0.22:
        return CLASS_WATER
    if g >= 195:
        return CLASS_WOODS_HEAVY if g >= 235 else CLASS_WOODS_LIGHT
    if r >= 185:
        return CLASS_RUBBLE if r >= 225 else CLASS_GROUND
    return CLASS_GRASS


def extract_single_package_draft(
    pkg_path,
    asset_suffix="",
    draft_id="extracted_arena",
    label="Extracted Arena",
    biome="temperate",
    body="luna",
    cols=256,
    rows=256,
    origin_m=(35000, 35000),
    elev_scale_m=60.0,
    raw_out_dir="",
):
    """Unpack a single `.unitypackage` TerrainData asset into a `ground.draft.v1` dict."""
    if not asset_suffix:
        candidates = list_unitypackage_terrain_assets(pkg_path)
        if not candidates:
            raise FileNotFoundError(f"No TerrainData `.asset` found inside `{pkg_path}`")
        asset_suffix = candidates[0][0]

    blob = extract_asset_from_unitypackage(pkg_path, asset_suffix)
    hm, hm_res = detect_unity_heightmap(blob)
    try:
        splat_bytes, sw, sh = parse_splat_rgba(blob)
    except ValueError:
        splat_bytes, sw, sh = None, 0, 0

    base = build_draft_dict([CLASS_GRASS] * (SIDE * SIDE), [0] * (SIDE * SIDE))
    road_class = CLASS_ROAD if biome in ("temperate", "mars") else CLASS_PAVED
    sx = cols / float(SIDE)
    sy = rows / float(SIDE)
    zone_depth = max(8, int(round(16 * sx)))

    flat_classes = []
    flat_elev = []

    for y in range(rows):
        v = y / float(max(1, rows - 1))
        by = min(SIDE - 1, int(y / sy))
        for x in range(cols):
            u = x / float(max(1, cols - 1))
            bx = min(SIDE - 1, int(x / sx))

            raw_h = sample_bilinear(hm, hm_res, u, v) / 32768.0
            edge_dist = min(
                max(0.0, x - float(zone_depth)),
                max(0.0, float(cols - 1 - zone_depth) - x),
            )
            edge_w = smoothstep(edge_dist / max(4.0, 14.0 * sx))
            elev_m = (raw_h - 0.25) * elev_scale_m * edge_w
            if x < zone_depth or x >= cols - zone_depth:
                q_cm = 0
            else:
                q_cm = int(round((elev_m * 100.0) / 50.0) * 50)
            flat_elev.append(max(-32768, min(32767, q_cm)))

            if (ROAD_EW[0] <= by <= ROAD_EW[1]) or (ROAD_NS[0] <= bx <= ROAD_NS[1]):
                flat_classes.append(road_class)
            elif x < zone_depth or x >= cols - zone_depth:
                if biome == "mars":
                    flat_classes.append(CLASS_SAND)
                elif biome == "lunar":
                    flat_classes.append(CLASS_REGOLITH)
                elif biome == "ice":
                    flat_classes.append(CLASS_ICE)
                else:
                    flat_classes.append(CLASS_GRASS)
            elif splat_bytes is not None:
                r, g, b, a = sample_splat_rgba(splat_bytes, sw, sh, u, v)
                flat_classes.append(splat_to_biome_class(biome, r, g, b, a, raw_h))
            else:
                flat_classes.append(splat_to_biome_class(biome, 0, 0, 0, 0, raw_h))

    if raw_out_dir:
        raw_dir = Path(raw_out_dir)
        raw_dir.mkdir(parents=True, exist_ok=True)
        le16_bytes = bytearray(len(flat_elev) * 2)
        for i, val in enumerate(flat_elev):
            struct.pack_into("<h", le16_bytes, i * 2, val)
        (raw_dir / f"{draft_id}.le16").write_bytes(le16_bytes)
        rle_b64 = encode_classes_rle(cols, rows, flat_classes)
        (raw_dir / f"{draft_id}.rle").write_bytes(base64.b64decode(rle_b64))

    def scale_xy(pt_x, pt_y):
        return (
            max(0, min(cols - 1, int(round(pt_x * sx)))),
            max(0, min(rows - 1, int(round(pt_y * sy)))),
        )

    scaled_spots = []
    for sp in base["key_spots"]:
        nx, ny = scale_xy(sp["x"], sp["y"])
        scaled_spots.append({"name": sp["name"], "x": nx, "y": ny})

    scaled_nodes = []
    for nd in base["waypoints"]["nodes"]:
        nx, ny = scale_xy(nd["x"], nd["y"])
        scaled_nodes.append({"id": nd["id"], "x": nx, "y": ny})

    scaled_tanks = []
    for tk in base["tanks"]:
        nx, ny = scale_xy(tk["x"], tk["y"])
        if tk["team"] == "red":
            nx = min(zone_depth - 2, max(1, nx))
        else:
            nx = max(cols - zone_depth + 1, min(cols - 2, nx))
        scaled_tanks.append(
            {
                "id": tk["id"],
                "name": tk["name"],
                "team": tk["team"],
                "x": nx,
                "y": ny,
                "heading_mdeg": tk["heading_mdeg"],
                "unit": tk["unit"],
            }
        )

    pkg_name = Path(pkg_path).name
    return {
        "schema": DRAFT_SCHEMA,
        "id": draft_id,
        "label": label,
        "biome": biome,
        "body": body,
        "origin_m": [int(origin_m[0]), int(origin_m[1])],
        "cell_cm": CELL_CM,
        "cols": cols,
        "rows": rows,
        "classes": encode_classes_rle(cols, rows, flat_classes),
        "elev": encode_elev_le16(flat_elev),
        "entry_zones": [
            {"team": "red", "x0": 0, "y0": 0, "x1": zone_depth - 1, "y1": rows - 1},
            {"team": "blue", "x0": cols - zone_depth, "y0": 0, "x1": cols - 1, "y1": rows - 1},
        ],
        "key_spots": scaled_spots,
        "waypoints": {
            "nodes": scaled_nodes,
            "edges": base["waypoints"]["edges"],
        },
        "modes": base["modes"],
        "tanks": scaled_tanks,
        "provenance": {
            "source": "extractor",
            "version": "1.0",
            "seed": 2350,
            "inputs": [f"{pkg_name} ({asset_suffix}, {hm_res}x{hm_res})"],
        },
    }


def build_synthetic_unitypackage(pkg_path, hm_res=65, splat_w=64):
    """Build a synthetic `.unitypackage` archive in `pkg_path` for `--self-test`."""
    count = hm_res * hm_res
    hm_samples = []
    for y in range(hm_res):
        for x in range(hm_res):
            wave = 0.25 + 0.15 * math.sin(x / 6.0) * math.cos(y / 6.0)
            hm_samples.append(int(max(0, min(65535, round(wave * 32768.0)))))
    hm_blob = struct.pack("<I", count) + struct.pack(f"<{count}H", *hm_samples)

    splat_sz = 16384
    splat_pixels = bytearray(splat_w * splat_w * 4)
    for y in range(splat_w):
        for x in range(splat_w):
            off = (y * splat_w + x) * 4
            splat_pixels[off] = 200 if x > splat_w * 3 // 4 else 40
            splat_pixels[off + 1] = 210 if y > splat_w * 2 // 3 else 80
            splat_pixels[off + 2] = 30
            splat_pixels[off + 3] = 255
    splat_header = (
        b"SplatAlpha\x00\x00"
        + struct.pack("<I", splat_sz)
        + b"\x00\x00\x00\x00"
        + struct.pack("<I", splat_sz)
    )
    asset_bytes = b"UnityTerrainAsset\x00" + splat_header + bytes(splat_pixels) + hm_blob

    guid = "0123456789abcdef0123456789abcdef"
    pathname_bytes = b"Assets/Terrains/SyntheticMarsTerrain.asset\n"

    with tarfile.open(pkg_path, "w:gz") as tf:
        p_info = tarfile.TarInfo(name=f"{guid}/pathname")
        p_info.size = len(pathname_bytes)
        tf.addfile(p_info, io.BytesIO(pathname_bytes))

        a_info = tarfile.TarInfo(name=f"{guid}/asset")
        a_info.size = len(asset_bytes)
        tf.addfile(a_info, io.BytesIO(asset_bytes))


def run_self_test():
    """Verify `.unitypackage` asset listing, extraction, RLE/le16 encoding, and provenance."""
    with tempfile.TemporaryDirectory() as tmp:
        pkg_path = Path(tmp) / "SyntheticMars.unitypackage"
        build_synthetic_unitypackage(pkg_path)

        assets = list_unitypackage_terrain_assets(pkg_path)
        assert len(assets) == 1, f"expected 1 TerrainData asset, got {assets!r}"
        assert assets[0][0].endswith("SyntheticMarsTerrain.asset")
        assert assets[0][1] == 65
        assert assets[0][2] == "64x64"

        raw_dir = Path(tmp) / "raw"
        draft = extract_single_package_draft(
            pkg_path,
            draft_id="synth_mars",
            label="Synthetic Mars",
            biome="mars",
            body="mars",
            cols=128,
            rows=128,
            origin_m=(45000, 45000),
            elev_scale_m=40.0,
            raw_out_dir=str(raw_dir),
        )
        assert draft["schema"] == DRAFT_SCHEMA
        assert draft["id"] == "synth_mars"
        assert draft["biome"] == "mars"
        assert draft["body"] == "mars"
        assert draft["cols"] == 128 and draft["rows"] == 128
        assert draft["provenance"]["source"] == "extractor"
        assert "SyntheticMars.unitypackage" in draft["provenance"]["inputs"][0]
        assert (raw_dir / "synth_mars.le16").stat().st_size == 128 * 128 * 2
        assert (raw_dir / "synth_mars.rle").is_file()
        print("extract.py --self-test passed")


def main():
    parser = argparse.ArgumentParser(
        description="Extract Unity heightmaps & splatmaps to ground.draft.v1 JSON."
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run built-in synthetic .unitypackage unpacker test and exit.",
    )
    parser.add_argument(
        "--list-assets",
        default="",
        help="List all TerrainData .asset entries inside a .unitypackage archive and exit.",
    )
    parser.add_argument(
        "--package",
        default="",
        help="Single .unitypackage archive to unpack into a ground.draft.v1 JSON.",
    )
    parser.add_argument(
        "--asset-suffix",
        default="",
        help="Target .asset suffix inside --package (auto-selects first TerrainData if omitted).",
    )
    parser.add_argument("--id", default="extracted_arena", help="Draft id for --package mode.")
    parser.add_argument("--label", default="Extracted Arena", help="Draft label for --package mode.")
    parser.add_argument(
        "--biome",
        default="temperate",
        choices=("temperate", "mars", "lunar", "ice"),
        help="Biome for --package mode.",
    )
    parser.add_argument("--body", default="luna", help="Parent body id for --package mode.")
    parser.add_argument("--cols", type=int, default=256, help="Output grid columns (64..512).")
    parser.add_argument("--rows", type=int, default=256, help="Output grid rows (64..512).")
    parser.add_argument(
        "--origin-m",
        default="35000,35000",
        help="Region origin_m as X,Y in metres for --package mode.",
    )
    parser.add_argument(
        "--elev-scale-m",
        type=float,
        default=60.0,
        help="Vertical elevation scale in metres for --package mode.",
    )
    parser.add_argument(
        "--raw-out-dir",
        default="",
        help="Optional directory to also emit raw <id>.le16 and <id>.rle files.",
    )
    parser.add_argument(
        "--unitypackage-dir",
        default=os.environ.get("UNITY_ASSETS_DIR", "C:/Users/faeni/3d_assets"),
        help="Root directory containing external .unitypackage archives for winter_valley stitch.",
    )
    parser.add_argument(
        "--extracted-dir",
        default=os.environ.get("UNITY_EXTRACTED_DIR", ""),
        help="Optional directory with pre-extracted .asset files for winter_valley stitch.",
    )
    parser.add_argument(
        "--out",
        default="",
        help="Output path for the generated ground.draft.v1 JSON.",
    )
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        return

    if args.list_assets:
        for internal_path, hm_res, splat_info in list_unitypackage_terrain_assets(args.list_assets):
            print(f"{internal_path}\theightmap={hm_res}x{hm_res}\tsplat={splat_info}")
        return

    if not args.out:
        parser.error("--out is required unless --self-test or --list-assets is given")

    if args.package:
        ox_s, oy_s = args.origin_m.split(",", 1)
        draft = extract_single_package_draft(
            args.package,
            asset_suffix=args.asset_suffix,
            draft_id=args.id,
            label=args.label,
            biome=args.biome,
            body=args.body,
            cols=args.cols,
            rows=args.rows,
            origin_m=(int(ox_s), int(oy_s)),
            elev_scale_m=args.elev_scale_m,
            raw_out_dir=args.raw_out_dir,
        )
    else:
        flat_classes, flat_elev = stitch_winter_valley(args.unitypackage_dir, args.extracted_dir)
        draft = build_draft_dict(flat_classes, flat_elev)

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(draft, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote extractor draft `{draft['id']}` to {out_path}")


if __name__ == "__main__":
    main()
