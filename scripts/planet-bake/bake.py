#!/usr/bin/env python3
"""Bake compact 128x64 equirectangular planet maps from `Solar System Planet Creator.unitypackage`.

Run in isolated mode (`python -I`, Python 3 standard library only):

1. Bake all 12 solar-system planetary maps into `crates/mush-pixel/planets/`:

    python -I scripts/planet-bake/bake.py \\
        --unitypackage-dir "C:/Users/faeni/3d_assets" \\
        --out-dir crates/mush-pixel/planets

2. Inspect planetary texture maps inside a `.unitypackage` archive:

    python -I scripts/planet-bake/bake.py \\
        --list-assets "C:/Users/faeni/3d_assets/galactica studio/3D ModelsEnvironmentsSci-Fi/Solar System Planet Creator.unitypackage"

3. Run the built-in self-test (no external `.unitypackage` files required):

    python -I scripts/planet-bake/bake.py --self-test

Licensing & repository hygiene (doc 22 / doc 30 / doc 35):
- Reads the external `.unitypackage` archive in memory outside the repository.
- Never writes or commits the multi-megabyte 4K-12K source PNG textures into the repository.
- Emits only the reduced `128x64` 8-bit indexed-colour `.png` equirectangular maps (~9 KB each)
  and `crates/mush-pixel/planets/index.json` recording upstream provenance and SHA-256 digests.
"""

import argparse
import hashlib
import io
import json
import math
import struct
import tarfile
import tempfile
import zlib
from pathlib import Path

BAKED_W = 128
BAKED_H = 64
SCANLINE_BYTES = BAKED_H * (1 + BAKED_W)  # 64 * 129 = 8256 bytes

DEFAULT_REL_PACKAGE = (
    "galactica studio/3D ModelsEnvironmentsSci-Fi/Solar System Planet Creator.unitypackage"
)
ASSET_PACK_NAME = "Galactica Studio - Solar System Planet Creator"

# Special feature marker indices in 240..=255 (albedo uses 0..=239)
FEAT_JUPITER_SPOT = 240
FEAT_MARS_ICE = 241
FEAT_CERES_FACULA = 242
FEAT_EARTH_CLOUD = 243

# 6-step colour ramps matching `crates/mush-pixel/src/planet_shader.rs`
RAMPS = {
    "neptune": [
        (8, 16, 48),
        (16, 40, 108),
        (24, 72, 168),
        (36, 112, 216),
        (64, 156, 244),
        (140, 212, 255),
    ],
    "uranus": [
        (12, 32, 44),
        (28, 76, 96),
        (56, 132, 156),
        (88, 180, 200),
        (136, 216, 228),
        (192, 240, 248),
    ],
    "saturn": [
        (32, 24, 14),
        (76, 58, 32),
        (132, 104, 58),
        (184, 148, 88),
        (224, 188, 120),
        (248, 224, 168),
    ],
    "jupiter": [
        (36, 22, 16),
        (88, 52, 36),
        (148, 92, 60),
        (196, 136, 92),
        (228, 184, 140),
        (248, 228, 196),
    ],
    "mars": [
        (36, 14, 10),
        (84, 32, 20),
        (140, 56, 32),
        (188, 84, 48),
        (220, 124, 76),
        (244, 188, 148),
    ],
    "earth": [
        (10, 24, 56),
        (20, 56, 124),
        (36, 104, 188),
        (56, 148, 112),
        (164, 204, 236),
        (240, 248, 255),
    ],
    "titan": [
        (36, 20, 8),
        (84, 48, 16),
        (144, 84, 24),
        (196, 124, 36),
        (232, 164, 64),
        (120, 196, 232),
    ],
    "triton": [
        (28, 24, 32),
        (68, 56, 72),
        (116, 96, 116),
        (168, 144, 160),
        (212, 188, 200),
        (244, 228, 236),
    ],
    "io": [
        (44, 24, 8),
        (108, 60, 16),
        (176, 104, 24),
        (220, 168, 36),
        (244, 216, 64),
        (252, 244, 148),
    ],
    "europa": [
        (24, 32, 44),
        (64, 80, 100),
        (116, 136, 156),
        (168, 188, 204),
        (212, 224, 236),
        (244, 250, 255),
    ],
    "ceres": [
        (20, 22, 26),
        (52, 56, 64),
        (92, 98, 108),
        (136, 142, 152),
        (180, 186, 196),
        (228, 232, 240),
    ],
    "luna": [
        (20, 22, 26),
        (52, 56, 64),
        (92, 98, 108),
        (136, 142, 152),
        (180, 186, 196),
        (228, 232, 240),
    ],
}

# Mapping from canonical baked body ID -> upstream `.unitypackage` texture path(s)
PLANET_SPECS = [
    ("neptune", "Assets/Solar System Planets/Texturemap/Solar System/Neptune_Tex.png", None),
    ("uranus", "Assets/Solar System Planets/Texturemap/Solar System/Uranus_Tex.png", None),
    ("saturn", "Assets/Solar System Planets/Texturemap/Solar System/Saturn_Tex.png", None),
    ("jupiter", "Assets/Solar System Planets/Texturemap/Solar System/Jupiter_Tex.png", None),
    ("mars", "Assets/Solar System Planets/Texturemap/Solar System/Mars_Tex.png", None),
    (
        "earth",
        "Assets/Solar System Planets/Texturemap/Solar System/Earth_Tex.png",
        "Assets/Solar System Planets/Texturemap/Solar System/cloud_Tex.png",
    ),
    ("titan", "Assets/Solar System Planets/Texturemap/Solar System/venus_Tex.png", None),
    ("triton", "Assets/Solar System Planets/Texturemap/Dwarf Planets/Eris_Tex.png", None),
    ("io", "Assets/Solar System Planets/Texturemap/Dwarf Planets/Makemake_Tex.png", None),
    ("europa", "Assets/Solar System Planets/Texturemap/Dwarf Planets/Haumea_Tex.png", None),
    ("ceres", "Assets/Solar System Planets/Texturemap/Dwarf Planets/Ceres_Tex.png", None),
    ("luna", "Assets/Solar System Planets/Texturemap/Solar System/Moon_Tex.png", None),
]


def parse_png_chunks(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Not a valid PNG signature")
    pos = 8
    w = h = bd = ct = ilace = 0
    idat = bytearray()
    while pos + 8 <= len(data):
        ln = struct.unpack(">I", data[pos : pos + 4])[0]
        tp = data[pos + 4 : pos + 8]
        chunk_data = data[pos + 8 : pos + 8 + ln]
        if tp == b"IHDR":
            w, h, bd, ct, _, _, ilace = struct.unpack(">IIBBBBB", chunk_data[:13])
        elif tp == b"IDAT":
            idat.extend(chunk_data)
        elif tp == b"IEND":
            break
        pos += 12 + ln
    return w, h, bd, ct, ilace, bytes(idat)


def paeth(a, b, c):
    p = a + b - c
    pa = abs(p - a)
    pb = abs(p - b)
    pc = abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    if pb <= pc:
        return b
    return c


def downsample_png_to_rgb_grid(png_bytes, out_w=BAKED_W, out_h=BAKED_H):
    """Decode an 8-bit PNG and downsample to an `out_w x out_h` grid of `(r, g, b)` tuples.

    - Fast path 1: If all sampled rows use PNG filter 0 (None) or 1 (Sub), only the `out_h`
      target rows are unfiltered in pure Python (no inter-row dependencies).
    - Fast path 2: For multi-megapixel images using vertical filters (2, 3, 4), uses Tcl/Tk's
      built-in C `PhotoImage` decoder when available, falling back to full pure-Python row
      reconstruction on headless environments.
    """
    w, h, bd, ct, ilace, idat = parse_png_chunks(png_bytes)
    if bd != 8 or ilace != 0:
        raise ValueError(f"Unsupported PNG format: bit_depth={bd}, interlace={ilace}")
    bpp = {0: 1, 2: 3, 4: 2, 6: 4}.get(ct)
    if bpp is None:
        raise ValueError(f"Unsupported PNG color_type={ct}")

    raw = zlib.decompress(idat)
    stride = w * bpp
    row_span = stride + 1

    sample_ys = [min(h - 1, int((vy + 0.5) * h / out_h)) for vy in range(out_h)]
    sample_xs = [min(w - 1, int((ux + 0.5) * w / out_w)) for ux in range(out_w)]

    all_filters = {raw[y * row_span] for y in range(h)}
    if all_filters.issubset({0, 1}):
        grid = []
        for y in sample_ys:
            r_start = y * row_span
            flt = raw[r_start]
            src = raw[r_start + 1 : r_start + row_span]
            if flt == 0:
                unfiltered = src
            else:
                buf = bytearray(stride)
                for i in range(stride):
                    left = buf[i - bpp] if i >= bpp else 0
                    buf[i] = (src[i] + left) & 0xFF
                unfiltered = buf
            row_rgb = []
            for x in sample_xs:
                off = x * bpp
                if bpp == 1 or bpp == 2:
                    v = unfiltered[off]
                    row_rgb.append((v, v, v))
                else:
                    row_rgb.append((unfiltered[off], unfiltered[off + 1], unfiltered[off + 2]))
            grid.append(row_rgb)
        return w, h, grid

    # If image is large and uses vertical filters (2, 3, 4), try C-accelerated tkinter PhotoImage first
    if w * h >= 2048 * 1024:
        try:
            import tkinter as tk

            root = getattr(downsample_png_to_rgb_grid, "_tk_root", None)
            if root is None:
                root = tk.Tk()
                root.withdraw()
                downsample_png_to_rgb_grid._tk_root = root
            img = tk.PhotoImage(data=png_bytes)
            sx = max(1, w // out_w)
            sy = max(1, h // out_h)
            sub = img.subsample(sx, sy)
            grid = []
            sw, sh = sub.width(), sub.height()
            for vy in range(out_h):
                yy = min(sh - 1, vy)
                row_rgb = []
                for ux in range(out_w):
                    xx = min(sw - 1, ux)
                    px = sub.get(xx, yy)
                    if isinstance(px, tuple):
                        row_rgb.append((int(px[0]), int(px[1]), int(px[2])))
                    else:
                        vals = [int(v) for v in str(px).split()]
                        row_rgb.append((vals[0], vals[1], vals[2]))
                grid.append(row_rgb)
            return w, h, grid
        except Exception:
            pass

    # Pure-Python full filter 0..4 reconstruction fallback
    prev = bytearray(stride)
    curr = bytearray(stride)
    target_set = set(sample_ys)
    decoded_rows = {}
    for y in range(h):
        r_start = y * row_span
        flt = raw[r_start]
        src = raw[r_start + 1 : r_start + row_span]
        if flt == 0:
            curr[:] = src
        elif flt == 1:
            for i in range(stride):
                a = curr[i - bpp] if i >= bpp else 0
                curr[i] = (src[i] + a) & 0xFF
        elif flt == 2:
            for i in range(stride):
                curr[i] = (src[i] + prev[i]) & 0xFF
        elif flt == 3:
            for i in range(stride):
                a = curr[i - bpp] if i >= bpp else 0
                b = prev[i]
                curr[i] = (src[i] + ((a + b) >> 1)) & 0xFF
        elif flt == 4:
            for i in range(stride):
                a = curr[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                curr[i] = (src[i] + paeth(a, b, c)) & 0xFF
        else:
            raise ValueError(f"Unknown PNG filter {flt}")
        if y in target_set:
            row_rgb = []
            for x in sample_xs:
                off = x * bpp
                if bpp in (1, 2):
                    v = curr[off]
                    row_rgb.append((v, v, v))
                else:
                    row_rgb.append((curr[off], curr[off + 1], curr[off + 2]))
            decoded_rows[y] = row_rgb
        prev, curr = curr, prev

    return w, h, [decoded_rows[y] for y in sample_ys]


def build_palette_for_body(body_id):
    """Build a 256-entry RGB palette (768 bytes) where:
    - indices `0..=239` interpolate smoothly across `RAMPS[body_id]`
    - indices `240..=255` hold special surface feature accent colours
    """
    ramp = RAMPS[body_id]
    n_seg = len(ramp) - 1
    plte = bytearray()
    for i in range(240):
        t = i / 239.0
        scaled = t * n_seg
        seg = min(n_seg - 1, int(scaled))
        frac = scaled - seg
        c0 = ramp[seg]
        c1 = ramp[seg + 1]
        r = int(round(c0[0] + (c1[0] - c0[0]) * frac))
        g = int(round(c0[1] + (c1[1] - c0[1]) * frac))
        b = int(round(c0[2] + (c1[2] - c0[2]) * frac))
        plte.extend((max(0, min(255, r)), max(0, min(255, g)), max(0, min(255, b))))

    # 240..=255 special feature slots
    accents = {
        FEAT_JUPITER_SPOT: (196, 68, 44),
        FEAT_MARS_ICE: (240, 244, 248),
        FEAT_CERES_FACULA: (228, 232, 240),
        FEAT_EARTH_CLOUD: (240, 248, 255),
    }
    for idx in range(240, 256):
        r, g, b = accents.get(idx, ramp[-1])
        plte.extend((r, g, b))
    return bytes(plte)


def quantize_equirect_grid(body_id, rgb_grid, cloud_grid=None):
    """Convert an `out_w x out_h` RGB grid into `0..=255` baked index bytes (`0..=239` albedo + `240..` features)."""
    lumas = []
    for row in rgb_grid:
        for r, g, b in row:
            luma = 0.299 * r + 0.587 * g + 0.114 * b
            lumas.append(luma)
    sorted_lumas = sorted(lumas)
    p02 = sorted_lumas[max(0, int(len(sorted_lumas) * 0.02))]
    p98 = sorted_lumas[min(len(sorted_lumas) - 1, int(len(sorted_lumas) * 0.98))]
    span = max(8.0, p98 - p02)

    indices = []
    for vy, row in enumerate(rgb_grid):
        lat_norm = (vy + 0.5) / BAKED_H * 2.0 - 1.0  # -1.0 (north) .. +1.0 (south)
        for ux, (r, g, b) in enumerate(row):
            luma = 0.299 * r + 0.587 * g + 0.114 * b
            if cloud_grid is not None:
                cr, cg, cb = cloud_grid[vy][ux]
                cloud_luma = (cr + cg + cb) / 3.0
                if cloud_luma >= 185:
                    indices.append(FEAT_EARTH_CLOUD)
                    continue
                luma = min(255.0, luma + cloud_luma * 0.45)

            norm = max(0.0, min(1.0, (luma - p02) / span))
            albedo_idx = int(round(norm * 239.0))

            # Planet-specific feature flags
            if body_id == "jupiter":
                # Great Red Spot & storm redness in southern tropical belt
                if 0.15 <= lat_norm <= 0.52 and (r - g) >= 38 and r >= 135:
                    indices.append(FEAT_JUPITER_SPOT)
                    continue
            elif body_id == "mars":
                # Polar ice cap detection near north/south poles
                if abs(lat_norm) >= 0.82 and luma >= p02 + span * 0.72 and (r - b) <= 45:
                    indices.append(FEAT_MARS_ICE)
                    continue
            elif body_id == "ceres":
                # Bright Occator/Ahuna faculae highlights
                if norm >= 0.96:
                    indices.append(FEAT_CERES_FACULA)
                    continue

            indices.append(max(0, min(239, albedo_idx)))
    return indices


def png_chunk(kind, data):
    crc = zlib.crc32(kind + data) & 0xFFFFFFFF
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc)


def encode_baked_png(body_id, indices):
    """Encode a `128x64` 8-bit indexed-colour PNG with a stored (`level=0`) IDAT block.

    Because `level=0` writes a single uncompressed RFC 1951 stored block (`8,256` scanline bytes
    at `idat[7 : 7 + 8256]`), the resulting file is both:
    1. A 100% standard colour PNG viewable in any browser, IDE, or `mush_pixel::png::decode_rgba`.
    2. Directly indexable in O(1) by `mush_pixel::planet_shader` in `no-default-features` WASM builds
       without requiring `flate2` at runtime.
    """
    if len(indices) != BAKED_W * BAKED_H:
        raise ValueError(f"Expected {BAKED_W * BAKED_H} pixels, got {len(indices)}")

    raw_scanlines = bytearray()
    for vy in range(BAKED_H):
        raw_scanlines.append(0)  # filter 0 (None)
        row_slice = indices[vy * BAKED_W : (vy + 1) * BAKED_W]
        raw_scanlines.extend(row_slice)

    if len(raw_scanlines) != SCANLINE_BYTES:
        raise AssertionError("Scanline length mismatch")

    idat_payload = zlib.compress(bytes(raw_scanlines), level=0)
    if idat_payload[7 : 7 + SCANLINE_BYTES] != raw_scanlines:
        raise AssertionError("zlib level=0 did not emit a contiguous stored block")

    ihdr = struct.pack(">IIBBBBB", BAKED_W, BAKED_H, 8, 3, 0, 0, 0)
    plte = build_palette_for_body(body_id)

    out = bytearray(b"\x89PNG\r\n\x1a\n")
    out.extend(png_chunk(b"IHDR", ihdr))
    out.extend(png_chunk(b"PLTE", plte))
    out.extend(png_chunk(b"IDAT", idat_payload))
    out.extend(png_chunk(b"IEND", b""))
    return bytes(out)


def load_package_textures(pkg_path):
    """Extract all `Texturemap/*.png` entries from a `.unitypackage` archive in memory."""
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

        textures = {}
        for guid, internal_path in pathnames.items():
            if internal_path.lower().endswith(".png") and guid in assets:
                f = tf.extractfile(assets[guid])
                if f is not None:
                    textures[internal_path] = f.read()
        return textures


def list_package_textures(pkg_path):
    textures = load_package_textures(pkg_path)
    rows = []
    for path in sorted(textures):
        try:
            w, h, bd, ct, ilace, _ = parse_png_chunks(textures[path])
            rows.append((path, w, h, bd, ct, len(textures[path])))
        except Exception:
            continue
    return rows


def bake_all(pkg_path, out_dir):
    pkg_path = Path(pkg_path)
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    pkg_sha = hashlib.sha256(pkg_path.read_bytes()).hexdigest()[:16]
    textures = load_package_textures(pkg_path)

    manifest_entries = []
    for body_id, tex_path, cloud_path in PLANET_SPECS:
        if tex_path not in textures:
            raise FileNotFoundError(f"Missing `{tex_path}` inside `{pkg_path}`")
        src_w, src_h, rgb_grid = downsample_png_to_rgb_grid(textures[tex_path])
        cloud_grid = None
        if cloud_path and cloud_path in textures:
            _, _, cloud_grid = downsample_png_to_rgb_grid(textures[cloud_path])

        indices = quantize_equirect_grid(body_id, rgb_grid, cloud_grid)
        baked_png = encode_baked_png(body_id, indices)
        out_file = out_dir / f"{body_id}.png"
        out_file.write_bytes(baked_png)
        file_sha = hashlib.sha256(baked_png).hexdigest()

        manifest_entries.append(
            {
                "id": body_id,
                "file": f"{body_id}.png",
                "width": BAKED_W,
                "height": BAKED_H,
                "bytes": len(baked_png),
                "sha256": file_sha,
                "source_texture": tex_path,
                "source_clouds": cloud_path,
                "source_dimensions": [src_w, src_h],
            }
        )
        print(
            f"baked {body_id:8s} <- {Path(tex_path).name:16s} ({src_w}x{src_h}) -> "
            f"{out_file.name} ({len(baked_png)} B, sha256:{file_sha[:12]})"
        )

    index_doc = {
        "schema": "mush.planet_bake.v1",
        "asset_pack": ASSET_PACK_NAME,
        "package_version": f"sha256:{pkg_sha}",
        "baked_dimensions": [BAKED_W, BAKED_H],
        "planets": manifest_entries,
    }
    index_path = out_dir / "index.json"
    index_path.write_text(json.dumps(index_doc, indent=2) + "\n", encoding="utf-8")
    return index_doc


def make_synthetic_png(w, h, pattern_seed):
    raw = bytearray()
    for y in range(h):
        raw.append(1)  # Filter 1 (Sub)
        prev_r = prev_g = prev_b = 0
        for x in range(w):
            r = int(128 + 90 * math.sin(x * 0.2 + pattern_seed) * math.cos(y * 0.25)) & 0xFF
            g = int(110 + 70 * math.cos(y * 0.3 + pattern_seed)) & 0xFF
            b = int(95 + 60 * math.sin((x + y) * 0.15)) & 0xFF
            raw.extend(((r - prev_r) & 0xFF, (g - prev_g) & 0xFF, (b - prev_b) & 0xFF))
            prev_r, prev_g, prev_b = r, g, b
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    idat = zlib.compress(bytes(raw), level=6)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", ihdr)
        + png_chunk(b"IDAT", idat)
        + png_chunk(b"IEND", b"")
    )


def run_self_test():
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        pkg_path = tmp_path / "SyntheticPlanets.unitypackage"
        all_paths = []
        for _, tex_path, cloud_path in PLANET_SPECS:
            all_paths.append(tex_path)
            if cloud_path:
                all_paths.append(cloud_path)

        with tarfile.open(pkg_path, "w:gz") as tf:
            for idx, internal_path in enumerate(sorted(set(all_paths))):
                guid = f"{idx:032x}"
                p_bytes = internal_path.encode("utf-8")
                p_info = tarfile.TarInfo(name=f"{guid}/pathname")
                p_info.size = len(p_bytes)
                tf.addfile(p_info, io.BytesIO(p_bytes))

                png_bytes = make_synthetic_png(64, 32, idx + 1)
                a_info = tarfile.TarInfo(name=f"{guid}/asset")
                a_info.size = len(png_bytes)
                tf.addfile(a_info, io.BytesIO(png_bytes))

        out_dir = tmp_path / "planets"
        doc = bake_all(pkg_path, out_dir)
        assert doc["schema"] == "mush.planet_bake.v1"
        assert len(doc["planets"]) == len(PLANET_SPECS)
        for entry in doc["planets"]:
            p_file = out_dir / entry["file"]
            assert p_file.is_file()
            w, h, bd, ct, ilace, idat = parse_png_chunks(p_file.read_bytes())
            assert (w, h, bd, ct, ilace) == (BAKED_W, BAKED_H, 8, 3, 0)
            inflated = zlib.decompress(idat)
            assert len(inflated) == SCANLINE_BYTES
            assert idat[7 : 7 + SCANLINE_BYTES] == inflated
    print("self-test: OK")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--unitypackage-dir",
        default="C:/Users/faeni/3d_assets",
        help="Root directory holding external Unity asset packages",
    )
    parser.add_argument(
        "--package",
        default="",
        help="Explicit path to `Solar System Planet Creator.unitypackage`",
    )
    parser.add_argument(
        "--out-dir",
        default="crates/mush-pixel/planets",
        help="Output directory for baked 128x64 planet PNGs and index.json",
    )
    parser.add_argument(
        "--list-assets",
        default="",
        help="List all planetary PNG textures inside the given `.unitypackage` archive",
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
        for path, w, h, bd, ct, sz in list_package_textures(args.list_assets):
            print(f"{path}: {w}x{h} (bit_depth={bd}, color_type={ct}, {sz // 1024} KiB)")
        return

    pkg_path = Path(args.package) if args.package else Path(args.unitypackage_dir) / DEFAULT_REL_PACKAGE
    if not pkg_path.is_file():
        raise FileNotFoundError(
            f"Could not find `{pkg_path}`; pass --package or --unitypackage-dir"
        )
    bake_all(pkg_path, args.out_dir)


if __name__ == "__main__":
    main()
