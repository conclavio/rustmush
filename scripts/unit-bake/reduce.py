"""Throwaway PoC: reduce the Unity bake to doc-22-style sheets at 48 and 96 px
(eight facings, still + four moving + a wreck per facing) and to voxel grids of
material classes. Usage: python reduce.py <bake_out> <poc_data>"""
import json, sys, os, zlib
import numpy as np
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
meta = json.load(open(os.path.join(src, 'meta.json')))
RES = meta['res']
SPAN_M = meta['span_m']
FACINGS = ['front', 'front_right', 'right', 'rear_right', 'rear', 'rear_left', 'left', 'front_left']
POSES = ['still', 'moving0', 'moving1', 'moving2', 'moving3', 'wreck']

# Colour-ID bits (r4 g2 b1) -> a part class.
#   green: main paint, blue: secondary panels, black: dark metal,
#   red: accent stripes, yellow: hazard, cyan: glass, magenta/white: lights.
PAINT, SECOND, DARK, ACCENT, BRIGHT = 1, 2, 3, 4, 5
BITS_TO_CLASS = {0: DARK, 1: SECOND, 2: PAINT, 3: BRIGHT, 4: ACCENT, 5: BRIGHT, 6: BRIGHT, 7: BRIGHT}

# Viewer-relative light: upper left, toward the viewer (camera looks +Z).
L = np.array([-0.4, 0.8, -0.45]); L /= np.linalg.norm(L)
CHARS = {'team-dark': 'a', 'team': 'b', 'team-light': 'c', 'black': 'k', 'steel': 's', 'pale': 'p'}


def classify(id_png, n_png):
    a = np.asarray(Image.open(id_png).convert('RGBA')).astype(int)
    n = np.asarray(Image.open(n_png).convert('RGB')).astype(float) / 255 * 2 - 1
    alpha = a[..., 3] > 127
    bits = (a[..., 0] > 127) * 4 + (a[..., 1] > 127) * 2 + (a[..., 2] > 127)
    lit = np.clip(n @ L, 0, 1)
    out = np.full(alpha.shape, '.', dtype='<U1')
    cls = np.vectorize(BITS_TO_CLASS.get)(bits)
    out[(cls == PAINT) & (lit > 0.72)] = 'c'
    out[(cls == PAINT) & (lit <= 0.72) & (lit > 0.35)] = 'b'
    out[(cls == PAINT) & (lit <= 0.35)] = 'a'
    out[(cls == SECOND) & (lit > 0.45)] = 's'
    out[(cls == SECOND) & (lit <= 0.45)] = 'a'
    out[cls == DARK] = 'k'
    out[cls == ACCENT] = 'c'
    out[cls == BRIGHT] = 'p'
    out[~alpha] = '.'
    return out


def reduce(hi, n):
    """Block-reduce to n x n: a cell is filled at >= 30% coverage (thin parts
    survive), its material the most common among its filled pixels."""
    k = RES // n
    blocks = hi.reshape(n, k, n, k).transpose(0, 2, 1, 3).reshape(n, n, k * k)
    rows = []
    for r in range(n):
        row = []
        for c in range(n):
            b = blocks[r, c]
            filled = b[b != '.']
            if len(filled) < 0.3 * k * k:
                row.append('.')
            else:
                v, cnt = np.unique(filled, return_counts=True)
                row.append(v[cnt.argmax()])
        rows.append(''.join(row))
    return rows


still = next(f for f in meta['frames'] if f['facing'] == 'front' and f['pose'] == 'still')
sizes = {}
for n in (48, 96):
    k = RES / n
    sheet = {
        'size': [n, n],
        'size_cm': [round(SPAN_M * 100), round(SPAN_M * 100)],
        'anchor': [int(still['anchor'][0] / k), min(n - 1, int(still['anchor'][1] / k))],
        'palette': {v: m for m, v in CHARS.items()},
        'facings': {},
    }
    for f in FACINGS:
        frames = {}
        for p in POSES:
            hi = classify(os.path.join(src, f'{f}_{p}_id.png'), os.path.join(src, f'{f}_{p}_n.png'))
            frames[p] = reduce(hi, n)
        m = next(x for x in meta['frames'] if x['facing'] == f and x['pose'] == 'still')
        muzzles = {w.split('_')[1]: [int(v[0] / k), int(v[1] / k)] for w, v in m['muzzles'].items()}
        sheet['facings'][f] = {
            'still': frames['still'],
            'moving': [frames[f'moving{i}'] for i in range(4)],
            'wreck': frames['wreck'],
            'muzzles': muzzles,
        }
    text = json.dumps(sheet, indent=1)
    open(os.path.join(dst, f'sheet{n}.json'), 'w').write(text)
    raw = text.encode()
    sizes[f'sheet{n}'] = {'json_bytes': len(raw), 'deflate_bytes': len(zlib.compress(raw, 9))}

    # Voxels: the bake's colour-ID bits + 1 -> a material class, 0 empty.
    total_raw = total_z = total_solid = 0
    for p in POSES:
        g = np.fromfile(os.path.join(src, f'vox{n}_{p}.bin'), np.uint8)
        lut = np.zeros(9, np.uint8)
        for bits, c in BITS_TO_CLASS.items():
            lut[bits + 1] = c
        m = lut[g]
        m.tofile(os.path.join(dst, f'vox{n}_{p}.bin'))
        total_raw += m.size
        total_z += len(zlib.compress(m.tobytes(), 9))
        total_solid += int((m > 0).sum())
    sizes[f'vox{n}'] = {'dense_bytes': total_raw, 'deflate_bytes': total_z, 'solid_voxels': total_solid,
                        'sparse_bytes_4b_each': total_solid * 4}

json.dump({'span_m': SPAN_M, 'height_m': meta['height_m'], 'sizes': sizes}, open(os.path.join(dst, 'sizes.json'), 'w'), indent=1)
print(json.dumps(sizes, indent=1))

# A preview: every facing's still, moving1 and wreck at both sizes, scaled up.
pal = {'.': (40, 44, 40), 'a': (120, 36, 32), 'b': (178, 54, 48), 'c': (214, 98, 84), 'k': (18, 18, 22), 's': (96, 100, 108), 'p': (216, 216, 198)}
for n in (48, 96):
    sheet = json.load(open(os.path.join(dst, f'sheet{n}.json')))
    z = 192 // n
    W = Image.new('RGB', (192 * 8, 192 * 3), pal['.'])
    for i, f in enumerate(FACINGS):
        for j, rows in enumerate([sheet['facings'][f]['still'], sheet['facings'][f]['moving'][1], sheet['facings'][f]['wreck']]):
            im = Image.new('RGB', (n, n))
            im.putdata([pal[ch] for row in rows for ch in row])
            W.paste(im.resize((n * z, n * z), Image.NEAREST), (i * 192, j * 192))
    W.save(os.path.join(dst, f'preview_sheet{n}.png'))
