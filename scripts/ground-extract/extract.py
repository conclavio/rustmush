#!/usr/bin/env python3
"""Extract and stitch Unity terrain heightmaps and splatmaps into a `ground.draft.v1` JSON.

Run in isolated mode (`python -I`, stdlib only):

    python -I scripts/ground-extract/extract.py \
        --unitypackage-dir "C:/Users/faeni/3d_assets" \
        --out crates/tacsim/content/ground/winter_valley.draft.json

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
import json
import math
import os
import struct
import tarfile
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


def parse_splat_rgba(data):
    pos = data.find(b"SplatAlpha")
    if pos < 0:
        raise ValueError("Could not find SplatAlpha in Unity TerrainData asset")
    window = data[pos : pos + 200]
    for sz, w in ((1398100, 512), (5592404, 1024), (22369620, 2048)):
        needle = struct.pack("<I", sz)
        p1 = window.find(needle)
        if p1 != -1:
            p2 = window.find(needle, p1 + 4)
            if p2 != -1:
                pix_start = pos + p2 + 4
                return data[pix_start : pix_start + w * w * 4], w, w
    raise ValueError("Could not locate SplatAlpha pixel buffer")


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


def main():
    parser = argparse.ArgumentParser(description="Extract Unity heightmaps & splatmaps to ground.draft.v1 JSON.")
    parser.add_argument(
        "--unitypackage-dir",
        default=os.environ.get("UNITY_ASSETS_DIR", "C:/Users/faeni/3d_assets"),
        help="Root directory containing external .unitypackage archives.",
    )
    parser.add_argument(
        "--extracted-dir",
        default=os.environ.get("UNITY_EXTRACTED_DIR", ""),
        help="Optional directory with pre-extracted .asset files.",
    )
    parser.add_argument(
        "--out",
        required=True,
        help="Output path for the generated ground.draft.v1 JSON.",
    )
    args = parser.parse_args()

    flat_classes, flat_elev = stitch_winter_valley(args.unitypackage_dir, args.extracted_dir)
    draft = build_draft_dict(flat_classes, flat_elev)
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(draft, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote extractor draft `{draft['id']}` to {out_path}")


if __name__ == "__main__":
    main()
