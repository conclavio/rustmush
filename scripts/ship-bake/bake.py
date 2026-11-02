#!/usr/bin/env python3
"""Bake 8-facing 256-colour shaded spaceship/asteroid sprites and 3D docking wireframes from MSGDI FBX packs.

Run in isolated mode (`python -I`, Python 3 standard library only):

1. Bake all 8 canonical space hull/object models into `crates/mush-pixel/ships/`:

    python -I scripts/ship-bake/bake.py \\
        --unitypackage-dir "C:/Users/faeni/3d_assets" \\
        --out-dir crates/mush-pixel/ships

2. List all `.fbx` meshes and vertex counts inside a `.unitypackage` archive:

    python -I scripts/ship-bake/bake.py \\
        --list-assets "C:/Users/faeni/3d_assets/MSGDI/3D ModelsVehiclesSpace/Spaceship Freighters.unitypackage"

3. Run the built-in synthetic `.unitypackage` self-test:

    python -I scripts/ship-bake/bake.py --self-test

Licensing & repository hygiene (doc 22 / doc 30 / doc 35):
- Reads external `.unitypackage` archives in memory outside the repository.
- Never writes or commits raw `.fbx` meshes or source textures into the repository.
- Emits only the reduced `512x64` (8 facings x `64x64`) 8-bit indexed-colour `.png` sprite sheets
  and `crates/mush-pixel/ships/index.json` (with provenance, SHA-256 digests, and decimated 3D
  feature-edge wireframes for the 6-DOF docking camera).
"""

import argparse
import hashlib
import io
import json
import math
import re
import struct
import tarfile
import tempfile
import zlib
from pathlib import Path

CELL_SIZE = 64
FACINGS = 8
SHEET_W = CELL_SIZE * FACINGS  # 512
SHEET_H = CELL_SIZE  # 64
SCANLINE_BYTES = SHEET_H * (1 + SHEET_W)  # 64 * 513 = 32,832 bytes (fits in 1 stored DEFLATE block)

IDX_TRANSPARENT = 0
IDX_OUTLINE = 240
IDX_PLUME = 241

# Canonical ship/object specs: (id, rel_package, fbx_suffix, asset_pack_label, pitch_deg, palette_ramp)
SHIP_SPECS = [
    (
        "freighter",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Freighters.unitypackage",
        "Assets/Freighters/FBX/FreighterClassII/FreighterClassII.fbx",
        "MSGDI - Spaceship Freighters",
        18.0,
        [(24, 30, 38), (52, 64, 80), (92, 110, 132), (140, 162, 188), (188, 208, 228), (232, 242, 252)],
    ),
    (
        "corvette",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Pirate Fleet Collection I.unitypackage",
        "Assets/PirateFleetMegaPackI/FBX/HammerheadClassPirateFrigate/HammerheadClassPirateFrigateHull.fbx",
        "MSGDI - Spaceship Pirate Fleet Collection I",
        18.0,
        [(20, 32, 44), (44, 72, 98), (78, 124, 164), (120, 174, 214), (172, 214, 242), (224, 244, 255)],
    ),
    (
        "raider",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Pirate Fleet Collection I.unitypackage",
        "Assets/PirateFleetMegaPackI/FBX/CorsairClassPirateHeavyFrigate/CorsairClassPirateHeavyFrigateHull.fbx",
        "MSGDI - Spaceship Pirate Fleet Collection I",
        18.0,
        [(38, 20, 20), (78, 38, 36), (128, 62, 56), (176, 92, 80), (216, 136, 116), (246, 196, 176)],
    ),
    (
        "drone",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Mining Operation.unitypackage",
        "Assets/MiningOperation/FBX/ResourceCollector/ResourceCollector.fbx",
        "MSGDI - Spaceship Mining Operation",
        22.0,
        [(36, 28, 16), (80, 62, 32), (136, 108, 52), (188, 152, 76), (224, 192, 112), (248, 228, 168)],
    ),
    (
        "torpedo",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Pirate Fleet Collection I.unitypackage",
        "Assets/PirateFleetMegaPackI/FBX/PirateMissileTorpedo/PirateTorpedo.fbx",
        "MSGDI - Spaceship Pirate Fleet Collection I",
        14.0,
        [(44, 18, 16), (96, 36, 32), (156, 58, 48), (208, 92, 72), (240, 148, 116), (255, 216, 192)],
    ),
    (
        "asteroid",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Mining Operation.unitypackage",
        "Assets/MiningOperation/FBX/Asteroids/Asteroid1.fbx",
        "MSGDI - Spaceship Mining Operation",
        20.0,
        [(26, 24, 22), (58, 54, 48), (98, 92, 82), (142, 134, 120), (184, 176, 160), (224, 216, 202)],
    ),
    (
        "derelict",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Shipwrecks.unitypackage",
        "Assets/Shipwrecks/FBX/DerelictLightDestroyer/DerelictLightDestroyer.fbx",
        "MSGDI - Spaceship Shipwrecks",
        20.0,
        [(22, 24, 28), (48, 52, 60), (82, 88, 100), (120, 128, 142), (160, 168, 182), (204, 212, 224)],
    ),
    (
        "shipyard",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Shipyards.unitypackage",
        "Assets/Shipyards/FBX/FrigateShipyard/FrigateShipyardHull.fbx",
        "MSGDI - Spaceship Shipyards",
        22.0,
        [(20, 28, 36), (46, 62, 78), (84, 108, 132), (132, 160, 186), (180, 206, 228), (228, 242, 252)],
    ),
    (
        "halcyon",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Carriers Fighters II.unitypackage",
        "Assets/Carrier+FighterFleetII/FBX/AttackCarrier/AttackCarrier.fbx",
        "MSGDI - Spaceship Carriers Fighters II",
        20.0,
        [(22, 34, 48), (48, 76, 108), (88, 132, 178), (136, 184, 226), (186, 220, 246), (234, 248, 255)],
    ),
    (
        "refinery",
        "MSGDI/3D ModelsVehiclesSpace/Spaceship Mining Operation.unitypackage",
        "Assets/MiningOperation/FBX/ResourceRefinery/ResourceRefinery.fbx",
        "MSGDI - Spaceship Mining Operation",
        22.0,
        [(32, 26, 18), (68, 56, 38), (114, 94, 62), (162, 136, 92), (206, 178, 128), (242, 222, 182)],
    ),
]


def parse_ascii_fbx(fbx_text):
    """Parse `Vertices:` and `PolygonVertexIndex:` from an ASCII FBX 6.1/7.x mesh."""
    v_match = re.search(r"Vertices:\s*\*?[0-9]*\s*\{?\s*(?:a:\s*)?([0-9eE+.\-,\s]+)", fbx_text)
    p_match = re.search(
        r"PolygonVertexIndex:\s*\*?[0-9]*\s*\{?\s*(?:a:\s*)?([0-9\-,\s]+)",
        fbx_text,
    )
    if not v_match or not p_match:
        raise ValueError("Could not find Vertices or PolygonVertexIndex in ASCII FBX")

    verts_raw = [float(x) for x in v_match.group(1).replace("\n", "").split(",") if x.strip()]
    polys_raw = [int(x) for x in p_match.group(1).replace("\n", "").split(",") if x.strip()]

    verts = [
        (verts_raw[i], verts_raw[i + 1], verts_raw[i + 2])
        for i in range(0, len(verts_raw) - 2, 3)
    ]

    faces = []
    current = []
    for idx in polys_raw:
        if idx < 0:
            real_idx = (-idx) - 1
            current.append(real_idx)
            if len(current) >= 3:
                faces.append(tuple(current))
            current = []
        else:
            current.append(idx)

    return verts, faces


def normalize_mesh(verts):
    """Center mesh at origin and normalize maximum half-extent to `1.0`."""
    xs = [v[0] for v in verts]
    ys = [v[1] for v in verts]
    zs = [v[2] for v in verts]
    cx = 0.5 * (min(xs) + max(xs))
    cy = 0.5 * (min(ys) + max(ys))
    cz = 0.5 * (min(zs) + max(zs))
    span = max(
        max(xs) - min(xs),
        max(ys) - min(ys),
        max(zs) - min(zs),
        1e-6,
    )
    scale = 2.0 / span
    return [((x - cx) * scale, (y - cy) * scale, (z - cz) * scale) for (x, y, z) in verts]


def face_normal(v0, v1, v2):
    ux, uy, uz = v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]
    vx, vy, vz = v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]
    nx = uy * vz - uz * vy
    ny = uz * vx - ux * vz
    nz = ux * vy - uy * vx
    ln = math.sqrt(nx * nx + ny * ny + nz * nz)
    if ln <= 1e-12:
        return (0.0, 1.0, 0.0)
    return (nx / ln, ny / ln, nz / ln)


def extract_feature_wireframe(verts, faces, max_edges=72):
    """Extract sharp crease and silhouette edges from a normalized mesh for the 6-DOF docking view.

    Returns a list of `[[x0_mm, y0_mm, z0_mm], [x1_mm, y1_mm, z1_mm]]` segments scaled to a
    `4,200 mm` hull envelope behind the docking collar (`z >= 200 mm`).
    """
    edge_faces = {}
    for face in faces:
        if len(face) < 3:
            continue
        n = face_normal(verts[face[0]], verts[face[1]], verts[face[2]])
        k = len(face)
        for i in range(k):
            a, b = face[i], face[(i + 1) % k]
            key = (min(a, b), max(a, b))
            edge_faces.setdefault(key, []).append(n)

    scored = []
    for (a, b), norms in edge_faces.items():
        va, vb = verts[a], verts[b]
        dx, dy, dz = vb[0] - va[0], vb[1] - va[1], vb[2] - va[2]
        length = math.sqrt(dx * dx + dy * dy + dz * dz)
        if length < 0.06:
            continue
        is_feature = False
        if len(norms) == 1:
            is_feature = True
        else:
            n0 = norms[0]
            for n1 in norms[1:]:
                dot = n0[0] * n1[0] + n0[1] * n1[1] + n0[2] * n1[2]
                if dot < 0.84:  # > ~32 deg crease angle
                    is_feature = True
                    break
        if is_feature:
            scored.append((length, a, b))

    scored.sort(key=lambda item: item[0], reverse=True)
    scale_mm = 3800.0
    z_offset_mm = 2600.0
    segments = []
    for _, a, b in scored[:max_edges]:
        va, vb = verts[a], verts[b]
        p0 = [
            int(round(va[0] * scale_mm)),
            int(round(va[1] * scale_mm)),
            int(round((va[2] + 1.0) * 0.5 * scale_mm + z_offset_mm)),
        ]
        p1 = [
            int(round(vb[0] * scale_mm)),
            int(round(vb[1] * scale_mm)),
            int(round((vb[2] + 1.0) * 0.5 * scale_mm + z_offset_mm)),
        ]
        segments.append([p0, p1])
    return segments


def rotate_vertex(v, yaw_rad, pitch_rad):
    x, y, z = v
    sy, cy = math.sin(yaw_rad), math.cos(yaw_rad)
    x1 = x * cy + z * sy
    z1 = -x * sy + z * cy
    y1 = y

    sp, cp = math.sin(pitch_rad), math.cos(pitch_rad)
    y2 = y1 * cp - z1 * sp
    z2 = y1 * sp + z1 * cp
    return (x1, y2, z2)


def render_facing_cell(verts, faces, yaw_deg, pitch_deg):
    """Software Z-buffered triangle rasterizer producing a `64x64` cell of index bytes (`0..=241`)."""
    yaw_rad = math.radians(yaw_deg)
    pitch_rad = math.radians(pitch_deg)
    r_verts = [rotate_vertex(v, yaw_rad, pitch_rad) for v in verts]

    # Sun + key light direction in camera space (upper-left-front)
    lx, ly, lz = 0.52, 0.58, -0.62

    zbuf = [[-1e9] * CELL_SIZE for _ in range(CELL_SIZE)]
    ibuf = [[IDX_TRANSPARENT] * CELL_SIZE for _ in range(CELL_SIZE)]

    half = (CELL_SIZE - 1) * 0.5
    scale_px = CELL_SIZE * 0.43

    for face in faces:
        if len(face) < 3:
            continue
        v0_orig = verts[face[0]]
        v1_orig = verts[face[1]]
        v2_orig = verts[face[2]]
        n_orig = face_normal(v0_orig, v1_orig, v2_orig)
        n_cam = rotate_vertex(n_orig, yaw_rad, pitch_rad)

        # Two-sided lighting magnitude so thin fins/plates always shade cleanly
        ndotl = abs(n_cam[0] * lx + n_cam[1] * ly + n_cam[2] * lz)
        rim = max(0.0, 1.0 - abs(n_cam[2])) * 0.25
        shade = max(0.08, min(1.0, 0.16 + 0.72 * ndotl + rim))

        # Check if this face is a stern engine nozzle (at stern Z < -0.75 facing -Z)
        cz_orig = (v0_orig[2] + v1_orig[2] + v2_orig[2]) / 3.0
        is_nozzle = cz_orig < -0.78 and n_orig[2] < -0.65 and n_cam[2] < 0.15
        if is_nozzle:
            idx_val = IDX_PLUME
        else:
            idx_val = max(1, min(239, int(round(1.0 + shade * 238.0))))

        # Fan triangulate polygon
        for k in range(1, len(face) - 1):
            p0 = r_verts[face[0]]
            p1 = r_verts[face[k]]
            p2 = r_verts[face[k + 1]]

            x0, y0, z0 = half + p0[0] * scale_px, half - p0[1] * scale_px, -p0[2]
            x1, y1, z1 = half + p1[0] * scale_px, half - p1[1] * scale_px, -p1[2]
            x2, y2, z2 = half + p2[0] * scale_px, half - p2[1] * scale_px, -p2[2]

            min_x = max(0, int(math.floor(min(x0, x1, x2))))
            max_x = min(CELL_SIZE - 1, int(math.ceil(max(x0, x1, x2))))
            min_y = max(0, int(math.floor(min(y0, y1, y2))))
            max_y = min(CELL_SIZE - 1, int(math.ceil(max(y0, y1, y2))))

            denom = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
            if abs(denom) < 1e-6:
                continue
            inv_denom = 1.0 / denom

            for py in range(min_y, max_y + 1):
                cy_px = py + 0.5
                for px in range(min_x, max_x + 1):
                    cx_px = px + 0.5
                    w0 = ((y1 - y2) * (cx_px - x2) + (x2 - x1) * (cy_px - y2)) * inv_denom
                    w1 = ((y2 - y0) * (cx_px - x2) + (x0 - x2) * (cy_px - y2)) * inv_denom
                    w2 = 1.0 - w0 - w1
                    if w0 >= -1e-4 and w1 >= -1e-4 and w2 >= -1e-4:
                        z = w0 * z0 + w1 * z1 + w2 * z2
                        if z > zbuf[py][px]:
                            zbuf[py][px] = z
                            ibuf[py][px] = idx_val

    # Add 1px dark hull silhouette outline around non-transparent pixels for crisp contrast
    out = [row[:] for row in ibuf]
    for py in range(1, CELL_SIZE - 1):
        for px in range(1, CELL_SIZE - 1):
            if ibuf[py][px] == IDX_TRANSPARENT:
                if (
                    ibuf[py - 1][px] > 0
                    or ibuf[py + 1][px] > 0
                    or ibuf[py][px - 1] > 0
                    or ibuf[py][px + 1] > 0
                ):
                    out[py][px] = IDX_OUTLINE
    return out


def build_ship_palette(ramp):
    """Build a 256-entry RGB palette (`PLTE`) + `tRNS` alpha table for a ship sprite sheet."""
    plte = bytearray()
    trns = bytearray()

    # Index 0: transparent
    plte.extend((0, 0, 0))
    trns.append(0)

    n_seg = len(ramp) - 1
    for i in range(1, 240):
        t = (i - 1) / 238.0
        scaled = t * n_seg
        seg = min(n_seg - 1, int(scaled))
        frac = scaled - seg
        c0 = ramp[seg]
        c1 = ramp[seg + 1]
        r = int(round(c0[0] + (c1[0] - c0[0]) * frac))
        g = int(round(c0[1] + (c1[1] - c0[1]) * frac))
        b = int(round(c0[2] + (c1[2] - c0[2]) * frac))
        plte.extend((max(0, min(255, r)), max(0, min(255, g)), max(0, min(255, b))))
        trns.append(255)

    # 240: dark silhouette outline, 241: engine plume cyan
    accents = {
        IDX_OUTLINE: (14, 18, 26),
        IDX_PLUME: (160, 228, 255),
    }
    for idx in range(240, 256):
        r, g, b = accents.get(idx, ramp[-1])
        plte.extend((r, g, b))
        trns.append(255)

    return bytes(plte), bytes(trns)


def png_chunk(kind, data):
    crc = zlib.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc)


def encode_ship_sheet_png(ramp, cells):
    """Encode 8 `64x64` facing cells into a `512x64` 8-bit indexed PNG with stored (`level=0`) IDAT."""
    raw_scanlines = bytearray()
    for py in range(SHEET_H):
        raw_scanlines.append(0)  # filter 0 (None)
        for f_idx in range(FACINGS):
            raw_scanlines.extend(cells[f_idx][py])

    if len(raw_scanlines) != SCANLINE_BYTES:
        raise AssertionError(f"Expected {SCANLINE_BYTES} scanline bytes, got {len(raw_scanlines)}")

    idat_payload = zlib.compress(bytes(raw_scanlines), level=0)
    if idat_payload[7 : 7 + SCANLINE_BYTES] != raw_scanlines:
        raise AssertionError("zlib level=0 did not emit a contiguous stored block")

    ihdr = struct.pack(">IIBBBBB", SHEET_W, SHEET_H, 8, 3, 0, 0, 0)
    plte, trns = build_ship_palette(ramp)

    out = bytearray(b"\x89PNG\r\n\x1a\n")
    out.extend(png_chunk(b"IHDR", ihdr))
    out.extend(png_chunk(b"PLTE", plte))
    out.extend(png_chunk(b"tRNS", trns))
    out.extend(png_chunk(b"IDAT", idat_payload))
    out.extend(png_chunk(b"IEND", b""))
    return bytes(out)


def extract_fbx_from_package(pkg_path, target_suffix):
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
                    pathnames[guid] = f.read().decode("utf-8", errors="replace").splitlines()[0].strip()
            elif leaf == "asset":
                assets[guid] = member

        for guid, internal_path in pathnames.items():
            if internal_path.endswith(target_suffix) and guid in assets:
                f = tf.extractfile(assets[guid])
                if f is not None:
                    return f.read().decode("utf-8", errors="replace")
    raise FileNotFoundError(f"Could not find `{target_suffix}` inside `{pkg_path}`")


def list_package_fbx(pkg_path):
    pathnames = {}
    assets = {}
    rows = []
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
                    pathnames[guid] = f.read().decode("utf-8", errors="replace").splitlines()[0].strip()
            elif leaf == "asset":
                assets[guid] = member
        for guid, internal_path in sorted(pathnames.items(), key=lambda kv: kv[1]):
            if internal_path.lower().endswith(".fbx") and guid in assets:
                f = tf.extractfile(assets[guid])
                if f is None:
                    continue
                txt = f.read().decode("utf-8", errors="replace")
                try:
                    verts, faces = parse_ascii_fbx(txt)
                    rows.append((internal_path, len(verts), len(faces), assets[guid].size))
                except Exception:
                    continue
    return rows


def bake_all(unitypackage_dir, out_dir):
    unitypackage_dir = Path(unitypackage_dir)
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    pkg_text_cache = {}
    pkg_sha_cache = {}
    manifest_entries = []

    for ship_id, rel_pkg, fbx_path, pack_label, pitch_deg, ramp in SHIP_SPECS:
        pkg_path = unitypackage_dir / rel_pkg
        if not pkg_path.is_file():
            raise FileNotFoundError(f"Missing package `{pkg_path}`")
        if rel_pkg not in pkg_sha_cache:
            pkg_sha_cache[rel_pkg] = hashlib.sha256(pkg_path.read_bytes()).hexdigest()[:16]
        cache_key = (rel_pkg, fbx_path)
        if cache_key not in pkg_text_cache:
            pkg_text_cache[cache_key] = extract_fbx_from_package(pkg_path, fbx_path)

        raw_verts, faces = parse_ascii_fbx(pkg_text_cache[cache_key])
        norm_verts = normalize_mesh(raw_verts)

        cells = [
            render_facing_cell(norm_verts, faces, f_idx * 45.0, pitch_deg)
            for f_idx in range(FACINGS)
        ]
        png_bytes = encode_ship_sheet_png(ramp, cells)
        out_file = out_dir / f"{ship_id}.png"
        out_file.write_bytes(png_bytes)
        file_sha = hashlib.sha256(png_bytes).hexdigest()

        wireframe = extract_feature_wireframe(norm_verts, faces, max_edges=64)

        manifest_entries.append(
            {
                "id": ship_id,
                "file": f"{ship_id}.png",
                "width": SHEET_W,
                "height": SHEET_H,
                "cell_size": CELL_SIZE,
                "facings": FACINGS,
                "bytes": len(png_bytes),
                "sha256": file_sha,
                "asset_pack": pack_label,
                "package_version": f"sha256:{pkg_sha_cache[rel_pkg]}",
                "source_fbx": fbx_path,
                "source_vertices": len(raw_verts),
                "source_faces": len(faces),
                "docking_wireframe_mm": wireframe,
            }
        )
        print(
            f"baked {ship_id:10s} <- {Path(fbx_path).name:38s} "
            f"({len(raw_verts):4d}v, {len(faces):4d}f) -> {out_file.name} "
            f"({len(png_bytes)} B, {len(wireframe)} edges, sha256:{file_sha[:12]})"
        )

    index_doc = {
        "schema": "mush.ship_bake.v1",
        "sheet_dimensions": [SHEET_W, SHEET_H],
        "cell_size": CELL_SIZE,
        "facings": FACINGS,
        "ships": manifest_entries,
    }
    (out_dir / "index.json").write_text(json.dumps(index_doc, indent=2) + "\n", encoding="utf-8")
    return index_doc


def make_synthetic_ascii_fbx(seed):
    s = 1.0 + 0.1 * seed
    verts = [
        (-1.0, -0.5, -2.0 * s),
        (1.0, -0.5, -2.0 * s),
        (1.0, 0.5, -2.0 * s),
        (-1.0, 0.5, -2.0 * s),
        (0.0, 0.0, 2.5 * s),
    ]
    v_str = ",".join(f"{x:.4f},{y:.4f},{z:.4f}" for (x, y, z) in verts)
    # 4 side triangles + 1 stern quad
    p_str = "0,1,-5,1,2,-5,2,3,-5,3,0,-5,0,3,2,-2"
    return f"; FBX 6.1.0 project file\nVertices: {v_str}\nPolygonVertexIndex: {p_str}\n"


def run_self_test():
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        by_pkg = {}
        for idx, (_, rel_pkg, fbx_path, _, _, _) in enumerate(SHIP_SPECS):
            by_pkg.setdefault(rel_pkg, []).append((idx, fbx_path))

        for rel_pkg, items in by_pkg.items():
            pkg_file = tmp_path / rel_pkg
            pkg_file.parent.mkdir(parents=True, exist_ok=True)
            with tarfile.open(pkg_file, "w:gz") as tf:
                for idx, fbx_path in items:
                    guid = f"{idx:032x}"
                    p_bytes = fbx_path.encode("utf-8")
                    p_info = tarfile.TarInfo(name=f"{guid}/pathname")
                    p_info.size = len(p_bytes)
                    tf.addfile(p_info, io.BytesIO(p_bytes))

                    fbx_bytes = make_synthetic_ascii_fbx(idx + 1).encode("utf-8")
                    a_info = tarfile.TarInfo(name=f"{guid}/asset")
                    a_info.size = len(fbx_bytes)
                    tf.addfile(a_info, io.BytesIO(fbx_bytes))

        out_dir = tmp_path / "ships_out"
        doc = bake_all(tmp_path, out_dir)
        assert doc["schema"] == "mush.ship_bake.v1"
        assert len(doc["ships"]) == len(SHIP_SPECS)
        for entry in doc["ships"]:
            f_path = out_dir / entry["file"]
            assert f_path.is_file()
            assert len(entry["docking_wireframe_mm"]) > 0
    print("self-test: OK")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--unitypackage-dir",
        default="C:/Users/faeni/3d_assets",
        help="Root directory holding external Unity asset packages",
    )
    parser.add_argument(
        "--out-dir",
        default="crates/mush-pixel/ships",
        help="Output directory for baked 512x64 ship sprite sheets and index.json",
    )
    parser.add_argument(
        "--list-assets",
        default="",
        help="List all ASCII `.fbx` meshes inside the given `.unitypackage` archive",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run built-in synthetic `.unitypackage` self-test",
    )
    args = parser.parse_args()

    if args.self_test:
        run_self_test()
        return

    if args.list_assets:
        for path, nv, nf, sz in list_package_fbx(args.list_assets):
            print(f"{path}: {nv} vertices, {nf} faces ({sz // 1024} KiB)")
        return

    bake_all(args.unitypackage_dir, args.out_dir)


if __name__ == "__main__":
    main()
