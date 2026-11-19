// `.mupk` Format v5 Binary Decoder & 3D Visual-Hull Mesh Carver (`web/src/lib/tacsim/mupk-mesh.ts`).
// Decodes shipped `mush-pixel` unit bakes (`MUPK` v5) directly in the browser, extracts the four
// orthogonal `fp` silhouettes (`Front`, `Right`, `Rear`, `Left`), and carves a single-draw-call
// 3D `THREE.BufferGeometry` with exact `shaded_rgb` material + 4-bit shade colors.

import * as THREE from "three";

export const MUPK_FORMAT_VERSION = 5;

export type RgbTriplet = readonly [number, number, number];

export interface TeamColorRamp {
  dark: RgbTriplet;
  mid: RgbTriplet;
  light: RgbTriplet;
}

export const RED_TEAM_RAMP: TeamColorRamp = {
  dark: [112, 30, 28],
  mid: [184, 54, 48],
  light: [236, 108, 92],
};

export const BLUE_TEAM_RAMP: TeamColorRamp = {
  dark: [26, 68, 122],
  mid: [48, 118, 196],
  light: [104, 178, 244],
};

export const AMBER_RIG_RAMP: TeamColorRamp = {
  dark: [118, 76, 20],
  mid: [194, 138, 46],
  light: [238, 188, 92],
};

export interface DecodedMupkFrame2D {
  sizePx: number;
  x: number;
  y: number;
  w: number;
  h: number;
  anchorX: number;
  anchorY: number;
  heightPx: number;
  /** Row-major `w * h` packed `(shade << 4) | materialCode` bytes (`0` = empty). */
  pixels: Uint8Array;
}

export interface DecodedMupkModel {
  modelId: string;
  assetPack: string;
  prefab: string;
  sizeCm: [number, number];
  spanCm: number;
  chosenSizePx: number;
  front: DecodedMupkFrame2D;
  right: DecodedMupkFrame2D;
  rear: DecodedMupkFrame2D;
  left: DecodedMupkFrame2D;
}

export interface CarvedMupkHullData {
  positions: Float32Array;
  normals: Float32Array;
  colors: Float32Array;
  solidVoxelCount: number;
  quadCount: number;
  boundsMeters: { widthX: number; heightY: number; depthZ: number };
}

// Vite asset URL map for shipped `.mupk` unit packs copied by `npm run wasm`.
const MUPK_ASSET_URLS = import.meta.glob<string>("../pixel/models/*.mupk", {
  query: "?url",
  import: "default",
  eager: true,
});

/** Resolve the Vite asset URL for a model id prefix (e.g. `"vf.mech.brawler_med"` or `"mc.vh.bug_rover"`). */
export function resolveMupkAssetUrl(modelIdPrefix: string): string | null {
  const needle = `/${modelIdPrefix}.`;
  for (const [key, url] of Object.entries(MUPK_ASSET_URLS)) {
    if (key.includes(needle) && key.endsWith(".mupk")) {
      return url;
    }
  }
  return null;
}

class MupkReader {
  readonly b: Uint8Array;
  at = 0;

  constructor(bytes: Uint8Array) {
    this.b = bytes;
  }

  take(n: number): Uint8Array {
    const end = this.at + n;
    if (n < 0 || end > this.b.length) {
      throw new Error("mupk pack ends early");
    }
    const slice = this.b.subarray(this.at, end);
    this.at = end;
    return slice;
  }

  u8(): number {
    if (this.at >= this.b.length) {
      throw new Error("mupk pack ends early");
    }
    return this.b[this.at++]!;
  }

  varint(): number {
    let val = 0;
    let factor = 1;
    for (let step = 0; step < 9; step++) {
      const byte = this.u8();
      val += (byte & 0x7f) * factor;
      if ((byte & 0x80) === 0) {
        return val;
      }
      factor *= 128;
    }
    throw new Error("mupk varint exceeds 64 bits");
  }

  str(): string {
    const len = this.varint();
    if (len > 4096) {
      throw new Error("mupk string exceeds 4096 bytes");
    }
    const raw = this.take(len);
    return new TextDecoder().decode(raw);
  }
}

export function decodeMupkRuns(rleBytes: Uint8Array, expectedPixels: number): Uint8Array {
  const out = new Uint8Array(expectedPixels);
  const r = new MupkReader(rleBytes);
  let writeAt = 0;
  while (r.at < rleBytes.length) {
    const pixelByte = r.u8();
    const matCode = pixelByte & 0x0f;
    if (matCode > 11) {
      throw new Error(`invalid mupk material code ${matCode}`);
    }
    if (matCode === 0 && pixelByte !== 0) {
      throw new Error("empty mupk pixel has non-zero shade");
    }
    const runLen = r.varint() + 1;
    if (writeAt + runLen > expectedPixels) {
      throw new Error("mupk runs overrun frame box");
    }
    if (pixelByte !== 0) {
      out.fill(pixelByte, writeAt, writeAt + runLen);
    }
    writeAt += runLen;
  }
  if (writeAt !== expectedPixels) {
    throw new Error(`mupk runs filled ${writeAt} of ${expectedPixels}`);
  }
  return out;
}

/**
 * Decode a `.mupk` Format v5 binary buffer and extract the 4 orthogonal `still` frames
 * (`Front`, `Right`, `Rear`, `Left`) from the `fp` chain at `preferredSizePx` (default `24`).
 * Skips non-orthogonal frames and larger mip levels in O(header) without decoding their RLE runs.
 */
export function decodeMupkModel(
  bytes: Uint8Array,
  preferredSizePx = 24,
): DecodedMupkModel | null {
  if (bytes.length < 7) return null;
  const r = new MupkReader(bytes);
  const magic = r.take(4);
  if (magic[0] !== 0x4d || magic[1] !== 0x55 || magic[2] !== 0x50 || magic[3] !== 0x4b) {
    throw new Error("not a mupk pack (missing MUPK magic)");
  }
  const version = r.u8() | (r.u8() << 8);
  if (version !== MUPK_FORMAT_VERSION) {
    throw new Error(`unsupported mupk version ${version} (expected ${MUPK_FORMAT_VERSION})`);
  }

  const entryCount = r.varint();
  if (entryCount === 0) return null;

  const modelId = r.str();
  const assetPack = r.str();
  const _assetVersion = r.str();
  const prefab = r.str();
  const clipCount = r.varint();
  for (let i = 0; i < clipCount; i++) r.str();
  const _bakeCommit = r.str();
  const _reduceCommit = r.str();
  const widthCm = r.varint();
  const heightCm = r.varint();
  const muzzleNameCount = r.varint();
  for (let i = 0; i < muzzleNameCount; i++) r.str();

  const chainCount = r.varint();
  for (let c = 0; c < chainCount; c++) {
    const viewTag = r.u8(); // 0 = fp, 1 = iso
    const spanCm = r.varint();
    const sizeCount = r.varint();

    // First scan or pick inside `fp` chain (`viewTag === 0`):
    // `FP_SIZES` is stored largest-first (`[192, 96, 64, 48, 32, 24, 16, 12, 8]`).
    let pickedFrames: Map<number, DecodedMupkFrame2D> | null = null;
    let pickedSizePx = 0;

    for (let s = 0; s < sizeCount; s++) {
      const sizePx = r.varint();
      const frameCount = r.varint();
      const wantThisSize: boolean =
        viewTag === 0 &&
        frameCount >= 42 &&
        (sizePx === preferredSizePx || (pickedFrames === null && sizePx <= preferredSizePx));

      const captured: Map<number, DecodedMupkFrame2D> | null = wantThisSize
        ? new Map<number, DecodedMupkFrame2D>()
        : null;

      for (let f = 0; f < frameCount; f++) {
        const head = r.take(7);
        const fx = head[0]!;
        const fy = head[1]!;
        const fw = head[2]!;
        const fh = head[3]!;
        const anchorX = head[4]!;
        const anchorY = head[5]!;
        const heightPx = head[6]!;

        const muzzleCount = r.varint();
        for (let m = 0; m < muzzleCount; m++) {
          r.varint();
          r.take(2);
        }

        const rleLen = r.varint();
        // Orthogonal `still` frames: facing 0 (`f=0`), 2 (`f=12`), 4 (`f=24`), 6 (`f=36`)
        if (captured !== null && (f === 0 || f === 12 || f === 24 || f === 36)) {
          const rleSlice = r.take(rleLen);
          const pixels = decodeMupkRuns(rleSlice, fw * fh);
          captured.set(f, {
            sizePx,
            x: fx,
            y: fy,
            w: fw,
            h: fh,
            anchorX,
            anchorY,
            heightPx,
            pixels,
          });
        } else {
          r.take(rleLen);
        }
      }

      if (captured !== null && captured.size === 4) {
        if (sizePx === preferredSizePx) {
          return {
            modelId,
            assetPack,
            prefab,
            sizeCm: [widthCm, heightCm],
            spanCm,
            chosenSizePx: sizePx,
            front: captured.get(0)!,
            right: captured.get(12)!,
            rear: captured.get(24)!,
            left: captured.get(36)!,
          };
        }
        pickedFrames = captured;
        pickedSizePx = sizePx;
      }
    }

    if (pickedFrames !== null && pickedFrames.size === 4) {
      return {
        modelId,
        assetPack,
        prefab,
        sizeCm: [widthCm, heightCm],
        spanCm,
        chosenSizePx: pickedSizePx,
        front: pickedFrames.get(0)!,
        right: pickedFrames.get(12)!,
        rear: pickedFrames.get(24)!,
        left: pickedFrames.get(36)!,
      };
    }
  }

  return null;
}

/** Exact port of `crates/mush-pixel/src/bake.rs::shaded_rgb` (`FORMAT_VERSION = 5`). */
export function shadedMupkRgb(pixelByte: number, team: TeamColorRamp): RgbTriplet {
  const code = pixelByte & 0x0f;
  const shade = Math.min(15, (pixelByte >>> 4) & 0x0f);

  const lerp = (a: RgbTriplet, b: RgbTriplet, t: number): RgbTriplet => [
    Math.round(a[0] + (b[0] - a[0]) * t),
    Math.round(a[1] + (b[1] - a[1]) * t),
    Math.round(a[2] + (b[2] - a[2]) * t),
  ];

  const ramp = (dark: RgbTriplet, mid: RgbTriplet, light: RgbTriplet): RgbTriplet => {
    const ink = lerp([0, 0, 0], dark, 0.55);
    return shade <= 7 ? lerp(ink, mid, shade / 7) : lerp(mid, light, (shade - 7) / 8);
  };

  const steelDark: RgbTriplet = [84, 88, 96];
  const steelMid: RgbTriplet = [118, 124, 132];
  const steelLight: RgbTriplet = [160, 164, 170];
  const rubberDark: RgbTriplet = [30, 29, 28];
  const rubberMid: RgbTriplet = [52, 50, 48];
  const rubberLight: RgbTriplet = [88, 86, 82];

  switch (code) {
    case 1:
      return team.dark;
    case 2:
      return ramp(team.dark, team.mid, team.light);
    case 3:
      return team.light;
    case 4:
      return [20, 22, 26];
    case 5:
      return ramp(steelDark, steelMid, steelLight);
    case 6:
      return [216, 216, 198];
    case 7:
      return steelDark;
    case 8:
      return steelLight;
    case 9:
      return ramp(rubberDark, rubberMid, rubberLight);
    case 10:
      return rubberLight;
    case 11:
      return [240, 216, 120];
    default:
      return [128, 134, 142];
  }
}

function sampleFrame(f: DecodedMupkFrame2D, duFromAnchor: number, dvAboveAnchor: number): number {
  const cx = f.anchorX + duFromAnchor;
  const cy = f.anchorY - dvAboveAnchor;
  if (cx < f.x || cx >= f.x + f.w || cy < f.y || cy >= f.y + f.h) {
    return 0;
  }
  return f.pixels[(cy - f.y) * f.w + (cx - f.x)]!;
}

/**
 * Carve a watertight 3D visual-hull mesh from the 4 orthogonal `.mupk` silhouettes,
 * culling all internal faces between adjacent solid voxels.
 */
export function carveMupkVisualHull(
  model: DecodedMupkModel,
  team: TeamColorRamp,
  targetHeightMeters: number,
): CarvedMupkHullData {
  const N = model.chosenSizePx;
  const half = Math.floor(N / 2);
  const gridW = N;
  const gridH = N + 4;
  const gridD = N;
  const padBelowAnchor = 2;

  const solid = new Uint8Array(gridW * gridH * gridD);
  const idxOf = (gx: number, gy: number, gz: number): number => (gy * gridD + gz) * gridW + gx;

  let minGy = gridH;
  let maxGy = -1;
  let minGx = gridW;
  let maxGx = -1;
  let minGz = gridD;
  let maxGz = -1;
  let solidCount = 0;

  for (let gy = 0; gy < gridH; gy++) {
    const dv = gy - padBelowAnchor;
    for (let gz = 0; gz < gridD; gz++) {
      const dz = gz - half;
      const pRight = sampleFrame(model.right, dz, dv);
      const pLeft = sampleFrame(model.left, -dz, dv);
      if (pRight === 0 && pLeft === 0) continue;

      for (let gx = 0; gx < gridW; gx++) {
        const dx = gx - half;
        const pFront = sampleFrame(model.front, -dx, dv);
        const pRear = sampleFrame(model.rear, dx, dv);
        if (pFront === 0 && pRear === 0) continue;

        solid[idxOf(gx, gy, gz)] = 1;
        solidCount++;
        if (gy < minGy) minGy = gy;
        if (gy > maxGy) maxGy = gy;
        if (gx < minGx) minGx = gx;
        if (gx > maxGx) maxGx = gx;
        if (gz < minGz) minGz = gz;
        if (gz > maxGz) maxGz = gz;
      }
    }
  }

  if (solidCount === 0) {
    return {
      positions: new Float32Array(0),
      normals: new Float32Array(0),
      colors: new Float32Array(0),
      solidVoxelCount: 0,
      quadCount: 0,
      boundsMeters: { widthX: 0, heightY: 0, depthZ: 0 },
    };
  }

  const spanY = Math.max(1, maxGy - minGy + 1);
  const voxelMeters = Math.max(0.03, targetHeightMeters / spanY);
  const centerGx = (minGx + maxGx + 1) * 0.5;
  const centerGz = (minGz + maxGz + 1) * 0.5;

  const posOut: number[] = [];
  const normOut: number[] = [];
  const colOut: number[] = [];
  let quadCount = 0;

  const isSolid = (gx: number, gy: number, gz: number): boolean => {
    if (gx < 0 || gx >= gridW || gy < 0 || gy >= gridH || gz < 0 || gz >= gridD) return false;
    return solid[idxOf(gx, gy, gz)] === 1;
  };

  const pushQuad = (
    v0: [number, number, number],
    v1: [number, number, number],
    v2: [number, number, number],
    v3: [number, number, number],
    nx: number,
    ny: number,
    nz: number,
    rgb: RgbTriplet,
  ): void => {
    const r = rgb[0] / 255;
    const g = rgb[1] / 255;
    const b = rgb[2] / 255;
    const triVerts = [v0, v1, v2, v0, v2, v3];
    for (const v of triVerts) {
      posOut.push(v[0], v[1], v[2]);
      normOut.push(nx, ny, nz);
      colOut.push(r, g, b);
    }
    quadCount++;
  };

  for (let gy = minGy; gy <= maxGy; gy++) {
    const dv = gy - padBelowAnchor;
    const y0 = (gy - minGy) * voxelMeters;
    const y1 = y0 + voxelMeters;

    for (let gz = minGz; gz <= maxGz; gz++) {
      const dz = gz - half;
      const z0 = (gz - centerGz) * voxelMeters;
      const z1 = z0 + voxelMeters;
      const pRight = sampleFrame(model.right, dz, dv);
      const pLeft = sampleFrame(model.left, -dz, dv);

      for (let gx = minGx; gx <= maxGx; gx++) {
        if (!isSolid(gx, gy, gz)) continue;
        const dx = gx - half;
        const x0 = (gx - centerGx) * voxelMeters;
        const x1 = x0 + voxelMeters;
        const pFront = sampleFrame(model.front, -dx, dv);
        const pRear = sampleFrame(model.rear, dx, dv);

        // +Z (Front)
        if (!isSolid(gx, gy, gz + 1)) {
          const px = pFront || pRight || pLeft || pRear;
          pushQuad([x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], 0, 0, 1, shadedMupkRgb(px, team));
        }
        // -Z (Rear)
        if (!isSolid(gx, gy, gz - 1)) {
          const px = pRear || pRight || pLeft || pFront;
          pushQuad([x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], 0, 0, -1, shadedMupkRgb(px, team));
        }
        // +X (Right)
        if (!isSolid(gx + 1, gy, gz)) {
          const px = pRight || pFront || pRear || pLeft;
          pushQuad([x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], 1, 0, 0, shadedMupkRgb(px, team));
        }
        // -X (Left)
        if (!isSolid(gx - 1, gy, gz)) {
          const px = pLeft || pFront || pRear || pRight;
          pushQuad([x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], -1, 0, 0, shadedMupkRgb(px, team));
        }
        // +Y (Top)
        if (!isSolid(gx, gy + 1, gz)) {
          const px = pFront || pRight || pRear || pLeft;
          pushQuad([x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0], 0, 1, 0, shadedMupkRgb(px, team));
        }
        // -Y (Bottom)
        if (!isSolid(gx, gy - 1, gz)) {
          const px = pRear || pRight || pLeft || pFront;
          pushQuad([x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1], 0, -1, 0, shadedMupkRgb(px, team));
        }
      }
    }
  }

  return {
    positions: new Float32Array(posOut),
    normals: new Float32Array(normOut),
    colors: new Float32Array(colOut),
    solidVoxelCount: solidCount,
    quadCount,
    boundsMeters: {
      widthX: (maxGx - minGx + 1) * voxelMeters,
      heightY: spanY * voxelMeters,
      depthZ: (maxGz - minGz + 1) * voxelMeters,
    },
  };
}

export function buildThreeMeshFromHullData(hull: CarvedMupkHullData): THREE.Mesh {
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.BufferAttribute(hull.positions, 3));
  geo.setAttribute("normal", new THREE.BufferAttribute(hull.normals, 3));
  geo.setAttribute("color", new THREE.BufferAttribute(hull.colors, 3));
  const mat = new THREE.MeshStandardMaterial({
    vertexColors: true,
    roughness: 0.48,
    metalness: 0.42,
  });
  return new THREE.Mesh(geo, mat);
}

/** Articulated procedural fallback chassis shown immediately while `.mupk` bytes stream in. */
export function buildArticulatedFallbackUnitGroup(
  archetype: "sim_mech" | "prospector_rover",
  team: TeamColorRamp,
): THREE.Group {
  const group = new THREE.Group();
  const bodyHex = (team.mid[0] << 16) | (team.mid[1] << 8) | team.mid[2];
  const trimHex = (team.light[0] << 16) | (team.light[1] << 8) | team.light[2];

  const armorMat = new THREE.MeshStandardMaterial({ color: bodyHex, roughness: 0.44, metalness: 0.56 });
  const darkMat = new THREE.MeshStandardMaterial({ color: 0x262d36, roughness: 0.68, metalness: 0.42 });
  const visorMat = new THREE.MeshStandardMaterial({
    color: trimHex,
    emissive: trimHex,
    emissiveIntensity: 0.85,
  });

  const addPart = (pos: [number, number, number], size: [number, number, number], mat: THREE.Material): THREE.Mesh => {
    const m = new THREE.Mesh(new THREE.BoxGeometry(size[0], size[1], size[2]), mat);
    m.position.set(pos[0], pos[1], pos[2]);
    group.add(m);
    return m;
  };

  if (archetype === "sim_mech") {
    // Articulated biped mech: feet, shins, pelvis, torso, shoulder missile pods, cockpit visor
    addPart([-0.46, 0.14, 0.08], [0.42, 0.28, 0.72], darkMat);
    addPart([0.46, 0.14, 0.08], [0.42, 0.28, 0.72], darkMat);
    addPart([-0.46, 0.66, -0.04], [0.32, 0.78, 0.38], armorMat);
    addPart([0.46, 0.66, -0.04], [0.32, 0.78, 0.38], armorMat);
    addPart([0, 1.12, 0], [1.05, 0.28, 0.68], darkMat);
    addPart([0, 1.62, 0.06], [1.26, 0.76, 0.96], armorMat);
    addPart([0, 1.72, 0.56], [0.76, 0.28, 0.12], visorMat);
    addPart([-0.82, 1.78, -0.02], [0.36, 0.44, 0.74], darkMat);
    addPart([0.82, 1.78, -0.02], [0.36, 0.44, 0.74], darkMat);
  } else {
    // Articulated 6-wheel prospector rover: bogie tracks, chassis tub, cab canopy, assay boom
    for (const side of [-1.06, 1.06]) {
      addPart([side, 0.36, 0], [0.38, 0.72, 3.85], darkMat);
    }
    addPart([0, 0.86, 0], [1.92, 0.88, 3.65], armorMat);
    addPart([0, 1.38, 0.88], [1.46, 0.46, 1.18], visorMat);
    addPart([0.42, 1.48, -0.95], [0.34, 0.38, 1.55], darkMat);
  }

  return group;
}

const decodedModelCache = new Map<string, Promise<DecodedMupkModel | null>>();

function fetchAndDecodeMupk(modelIdPrefix: string): Promise<DecodedMupkModel | null> {
  const existing = decodedModelCache.get(modelIdPrefix);
  if (existing) return existing;

  const url = resolveMupkAssetUrl(modelIdPrefix);
  if (!url || typeof fetch !== "function") {
    const empty = Promise.resolve(null);
    decodedModelCache.set(modelIdPrefix, empty);
    return empty;
  }

  const pending = fetch(url)
    .then(async (res) => {
      if (!res.ok) return null;
      const buf = await res.arrayBuffer();
      return decodeMupkModel(new Uint8Array(buf), 24);
    })
    .catch(() => null);

  decodedModelCache.set(modelIdPrefix, pending);
  return pending;
}

/**
 * Release every geometry and material under `obj` (48e L8, clients:G17). A
 * detached Three.js object keeps its GPU buffers until they are disposed, so a
 * swapped-out fallback chassis is disposed, not only removed. The fallback's
 * materials are made per unit, so nothing shared is released.
 */
export function disposeObjectTree(obj: THREE.Object3D): void {
  obj.traverse((node) => {
    const mesh = node as THREE.Mesh;
    if (!mesh.isMesh) return;
    mesh.geometry?.dispose();
    const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
    for (const m of mats) m?.dispose();
  });
}

/**
 * Mount a unit container (`THREE.Group`) holding an immediate articulated 3D chassis that
 * upgrades seamlessly in-place to the carved `.mupk` 3D visual-hull mesh once fetched.
 */
export function createAsyncMupkUnitGroup(
  modelIdPrefix: string,
  fallbackArchetype: "sim_mech" | "prospector_rover",
  team: TeamColorRamp,
  targetHeightMeters: number,
): { group: THREE.Group; hitTarget: THREE.Object3D } {
  const root = new THREE.Group();
  const fallback = buildArticulatedFallbackUnitGroup(fallbackArchetype, team);
  root.add(fallback);

  // Invisible raycast proxy box so `Stage3D` hover/click raycasts remain rock-solid across swap
  const hitGeo =
    fallbackArchetype === "sim_mech"
      ? new THREE.BoxGeometry(1.9, targetHeightMeters, 1.9)
      : new THREE.BoxGeometry(2.3, targetHeightMeters, 4.0);
  const hitMat = new THREE.MeshBasicMaterial({ transparent: true, opacity: 0, depthWrite: false });
  const hitTarget = new THREE.Mesh(hitGeo, hitMat);
  hitTarget.position.set(0, targetHeightMeters * 0.5, 0);
  root.add(hitTarget);

  void fetchAndDecodeMupk(modelIdPrefix).then((decoded) => {
    if (!decoded || !root.parent) return;
    const hull = carveMupkVisualHull(decoded, team, targetHeightMeters);
    if (hull.quadCount === 0) return;
    root.remove(fallback);
    disposeObjectTree(fallback);
    const mesh = buildThreeMeshFromHullData(hull);
    root.add(mesh);
  });

  return { group: root, hitTarget };
}
