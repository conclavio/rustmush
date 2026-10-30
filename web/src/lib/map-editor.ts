// Unity-style Terrain & Arena Map Studio engine: pure, DOM-free data structures,
// brush footprints, elevation sculpting with grade clamping, procedural base
// terrain generators, exact `mush_atlas::Cells` little-endian RLE base64
// encode/decode, `ArenaDef` JSON import/export, and live `mush-pixel-wasm`
// (`arena.map` + `arena.cockpit`) feed synthesis.

import type { ArenaSpot } from "./cockpit";

export type BiomeId = "temperate" | "mars" | "lunar" | "ice";

export type EditorTool =
  | "paint"
  | "raise"
  | "lower"
  | "flatten"
  | "smooth"
  | "ramp"
  | "line"
  | "rect"
  | "fill"
  | "eyedrop"
  | "spot"
  | "muster_red"
  | "muster_blue"
  | "camera";

export type BrushShape = "circle" | "square" | "diamond";

export interface TerrainMeta {
  code: number;
  id: string;
  glyph: string;
  label: string;
  category: "ground" | "foliage" | "water" | "structure" | "planetary";
}

/** Complete `mush_atlas::TerrainClass` table in byte-code order (0..17). */
export const EDITOR_TERRAINS: readonly TerrainMeta[] = [
  { code: 0, id: "void", glyph: " ", label: "Void", category: "ground" },
  { code: 1, id: "paved", glyph: ".", label: "Paved", category: "structure" },
  { code: 2, id: "ground", glyph: ",", label: "Dirt / Ground", category: "ground" },
  { code: 3, id: "grass", glyph: '"', label: "Grass", category: "ground" },
  { code: 4, id: "water", glyph: "~", label: "Shallow Water", category: "water" },
  { code: 5, id: "floor", glyph: "_", label: "Indoor Floor", category: "structure" },
  { code: 6, id: "rubble", glyph: "x", label: "Rubble", category: "ground" },
  { code: 7, id: "road", glyph: "=", label: "Road", category: "structure" },
  { code: 8, id: "sand", glyph: ":", label: "Sand", category: "planetary" },
  { code: 9, id: "rock", glyph: "^", label: "Rock / Ridge", category: "ground" },
  { code: 10, id: "wall", glyph: "#", label: "Wall", category: "structure" },
  { code: 11, id: "door", glyph: "+", label: "Door", category: "structure" },
  { code: 12, id: "platform", glyph: "D", label: "Platform", category: "structure" },
  { code: 13, id: "woods_light", glyph: "t", label: "Light Woods", category: "foliage" },
  { code: 14, id: "woods_heavy", glyph: "T", label: "Heavy Woods", category: "foliage" },
  { code: 15, id: "water_deep", glyph: "W", label: "Deep Water", category: "water" },
  { code: 16, id: "ice", glyph: "i", label: "Ice Sheet", category: "planetary" },
  { code: 17, id: "regolith", glyph: "r", label: "Regolith", category: "planetary" },
];

const BY_CODE = new Map<number, TerrainMeta>(EDITOR_TERRAINS.map((t) => [t.code, t]));
const BY_ID = new Map<string, TerrainMeta>(EDITOR_TERRAINS.map((t) => [t.id, t]));
const BY_GLYPH = new Map<string, TerrainMeta>(EDITOR_TERRAINS.map((t) => [t.glyph, t]));

export function terrainByCode(code: number): TerrainMeta {
  return BY_CODE.get(code) ?? EDITOR_TERRAINS[0]!;
}

export function terrainById(id: string): TerrainMeta {
  return BY_ID.get(id) ?? EDITOR_TERRAINS[3]!;
}

export function terrainByGlyph(glyph: string): TerrainMeta {
  return BY_GLYPH.get(glyph) ?? EDITOR_TERRAINS[0]!;
}

/** Maximum adjacent cell elevation delta (cm) to stay within the 4.5% tank grade limit
 * on 10m (`1000 cm`) cells. */
export const MAX_ADJACENT_DELTA_CM = 45;
export const MIN_ELEV_CM = -500;
export const MAX_ELEV_CM = 1200;

export interface MusterZone {
  rect: [number, number, number, number];
  headingDeg: number;
}

export interface EditorCamera {
  x: number;
  y: number;
  headingDeg: number;
}

export interface EditorMapState {
  id: string;
  label: string;
  note: string;
  biome: BiomeId;
  width: number;
  height: number;
  cellCm: number;
  /** Row-major from south-west (`y * width + x`), storing `TerrainClass` code (0..17). */
  classes: Uint8Array;
  /** Row-major from south-west (`y * width + x`), storing `elev_cm` in `[-500, 1200]`. */
  elevCm: Int16Array;
  redMuster: MusterZone;
  blueMuster: MusterZone;
  spots: ArenaSpot[];
  camera: EditorCamera;
}

export interface BrushPoint {
  x: number;
  y: number;
  /** Radial weight in `(0, 1]`, 1 at center and tapering toward the edge. */
  weight: number;
}

/** Clone an `EditorMapState` for undo/redo snapshots. */
export function cloneMapState(state: EditorMapState): EditorMapState {
  return {
    ...state,
    classes: new Uint8Array(state.classes),
    elevCm: new Int16Array(state.elevCm),
    redMuster: { rect: [...state.redMuster.rect], headingDeg: state.redMuster.headingDeg },
    blueMuster: { rect: [...state.blueMuster.rect], headingDeg: state.blueMuster.headingDeg },
    spots: state.spots.map((s) => ({ ...s, cell: [s.cell[0], s.cell[1]] })),
    camera: { ...state.camera },
  };
}

/** Compute the cells covered by a brush centered at `(cx, cy)` with `radius >= 1`. */
export function brushFootprint(
  cx: number,
  cy: number,
  radius: number,
  shape: BrushShape,
  width: number,
  height: number,
): BrushPoint[] {
  const r = Math.max(1, Math.min(12, Math.trunc(radius)));
  const span = r - 1;
  const out: BrushPoint[] = [];
  for (let dy = -span; dy <= span; dy += 1) {
    const y = cy + dy;
    if (y < 0 || y >= height) continue;
    for (let dx = -span; dx <= span; dx += 1) {
      const x = cx + dx;
      if (x < 0 || x >= width) continue;
      let dist = 0;
      if (shape === "circle") {
        dist = Math.hypot(dx, dy);
        if (dist > span + 0.35) continue;
      } else if (shape === "diamond") {
        dist = Math.abs(dx) + Math.abs(dy);
        if (dist > span) continue;
      } else {
        dist = Math.max(Math.abs(dx), Math.abs(dy));
      }
      const weight = span === 0 ? 1 : Math.max(0.2, 1 - dist / (span + 1));
      out.push({ x, y, weight });
    }
  }
  return out;
}

/** Deterministic integer hash for scatter brushes and procedural terrain. */
function hash2d(x: number, y: number, seed: number): number {
  let h = (Math.imul(x, 374761393) + Math.imul(y, 668265263) + Math.imul(seed, 1442695041)) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) % 1000;
}

/** Enforce the `<= 45 cm` adjacent cell grade limit in a local window `[x0..x1, y0..y1]`. */
export function enforceLocalGradeLimit(
  state: EditorMapState,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
): void {
  const { width, height, elevCm } = state;
  const minX = Math.max(0, x0 - 6);
  const maxX = Math.min(width - 1, x1 + 6);
  const minY = Math.max(0, y0 - 6);
  const maxY = Math.min(height - 1, y1 + 6);

  for (let pass = 0; pass < 4; pass += 1) {
    for (let y = minY; y <= maxY; y += 1) {
      for (let x = minX; x <= maxX; x += 1) {
        const idx = y * width + x;
        let v = elevCm[idx] ?? 0;
        if (x > 0) {
          const left = elevCm[idx - 1] ?? 0;
          if (v - left > MAX_ADJACENT_DELTA_CM) v = left + MAX_ADJACENT_DELTA_CM;
          else if (left - v > MAX_ADJACENT_DELTA_CM) v = left - MAX_ADJACENT_DELTA_CM;
        }
        if (y > 0) {
          const down = elevCm[idx - width] ?? 0;
          if (v - down > MAX_ADJACENT_DELTA_CM) v = down + MAX_ADJACENT_DELTA_CM;
          else if (down - v > MAX_ADJACENT_DELTA_CM) v = down - MAX_ADJACENT_DELTA_CM;
        }
        elevCm[idx] = Math.max(MIN_ELEV_CM, Math.min(MAX_ELEV_CM, v));
      }
    }
    for (let y = maxY; y >= minY; y -= 1) {
      for (let x = maxX; x >= minX; x -= 1) {
        const idx = y * width + x;
        let v = elevCm[idx] ?? 0;
        if (x + 1 < width) {
          const right = elevCm[idx + 1] ?? 0;
          if (v - right > MAX_ADJACENT_DELTA_CM) v = right + MAX_ADJACENT_DELTA_CM;
          else if (right - v > MAX_ADJACENT_DELTA_CM) v = right - MAX_ADJACENT_DELTA_CM;
        }
        if (y + 1 < height) {
          const up = elevCm[idx + width] ?? 0;
          if (v - up > MAX_ADJACENT_DELTA_CM) v = up + MAX_ADJACENT_DELTA_CM;
          else if (up - v > MAX_ADJACENT_DELTA_CM) v = up - MAX_ADJACENT_DELTA_CM;
        }
        elevCm[idx] = Math.max(MIN_ELEV_CM, Math.min(MAX_ELEV_CM, v));
      }
    }
  }
}

export interface BrushOptions {
  tool: EditorTool;
  terrainCode: number;
  radius: number;
  shape: BrushShape;
  /** Scatter density 10..100 (100 = solid paint). */
  densityPct: number;
  /** Elevation step per stroke (cm) for raise/lower, default 40. */
  elevStepCm: number;
  /** Target elevation (cm) for flatten tool. */
  targetElevCm: number;
  strokeSeed?: number;
}

/** Apply a single brush dab at `(cx, cy)`. */
export function applyBrushDab(
  state: EditorMapState,
  cx: number,
  cy: number,
  opts: BrushOptions,
): void {
  const { width, height, classes, elevCm } = state;
  const pts = brushFootprint(cx, cy, opts.radius, opts.shape, width, height);
  if (pts.length === 0) return;

  if (opts.tool === "paint") {
    const density = Math.max(10, Math.min(100, opts.densityPct)) * 10;
    const seed = opts.strokeSeed ?? 17;
    for (const p of pts) {
      if (density < 1000 && hash2d(p.x, p.y, seed) >= density) continue;
      classes[p.y * width + p.x] = opts.terrainCode;
    }
    return;
  }

  if (opts.tool === "raise" || opts.tool === "lower") {
    const sign = opts.tool === "raise" ? 1 : -1;
    const step = Math.max(5, Math.min(200, opts.elevStepCm));
    for (const p of pts) {
      const idx = p.y * width + p.x;
      const cur = elevCm[idx] ?? 0;
      const delta = Math.round(sign * step * p.weight);
      elevCm[idx] = Math.max(MIN_ELEV_CM, Math.min(MAX_ELEV_CM, cur + delta));
    }
    const r = opts.radius;
    enforceLocalGradeLimit(state, cx - r, cy - r, cx + r, cy + r);
    return;
  }

  if (opts.tool === "flatten") {
    const target = Math.max(MIN_ELEV_CM, Math.min(MAX_ELEV_CM, Math.round(opts.targetElevCm)));
    for (const p of pts) {
      const idx = p.y * width + p.x;
      const cur = elevCm[idx] ?? 0;
      elevCm[idx] = Math.round(cur + (target - cur) * p.weight);
    }
    const r = opts.radius;
    enforceLocalGradeLimit(state, cx - r, cy - r, cx + r, cy + r);
    return;
  }

  if (opts.tool === "smooth") {
    const copy = new Int16Array(elevCm);
    for (const p of pts) {
      let sum = 0;
      let weightSum = 0;
      for (let dy = -1; dy <= 1; dy += 1) {
        const ny = Math.max(0, Math.min(height - 1, p.y + dy));
        for (let dx = -1; dx <= 1; dx += 1) {
          const nx = Math.max(0, Math.min(width - 1, p.x + dx));
          const kw = dx === 0 && dy === 0 ? 4 : dx === 0 || dy === 0 ? 2 : 1;
          sum += (copy[ny * width + nx] ?? 0) * kw;
          weightSum += kw;
        }
      }
      const avg = Math.round(sum / weightSum);
      const idx = p.y * width + p.x;
      const cur = copy[idx] ?? 0;
      elevCm[idx] = Math.round(cur + (avg - cur) * p.weight);
    }
    const r = opts.radius;
    enforceLocalGradeLimit(state, cx - r, cy - r, cx + r, cy + r);
  }
}

/** Bresenham line cells between `(x0, y0)` and `(x1, y1)`. */
export function bresenhamCells(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
): [number, number][] {
  const out: [number, number][] = [];
  let cx = Math.trunc(x0);
  let cy = Math.trunc(y0);
  const tx = Math.trunc(x1);
  const ty = Math.trunc(y1);
  const dx = Math.abs(tx - cx);
  const sx = cx < tx ? 1 : -1;
  const dy = -Math.abs(ty - cy);
  const sy = cy < ty ? 1 : -1;
  let err = dx + dy;
  for (;;) {
    out.push([cx, cy]);
    if (cx === tx && cy === ty) break;
    const e2 = 2 * err;
    if (e2 >= dy) {
      err += dy;
      cx += sx;
    }
    if (e2 <= dx) {
      err += dx;
      cy += sy;
    }
  }
  return out;
}

/** Paint a line from `(x0, y0)` to `(x1, y1)` using the brush radius and terrain class. */
export function applyLineStroke(
  state: EditorMapState,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  opts: BrushOptions,
): void {
  const cells = bresenhamCells(x0, y0, x1, y1);
  for (const [x, y] of cells) {
    applyBrushDab(state, x, y, { ...opts, tool: "paint", densityPct: 100 });
  }
}

/** Carve a graded linear elevation ramp from `(x0, y0)` to `(x1, y1)`. */
export function applyRampStroke(
  state: EditorMapState,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  radius: number,
  paintTerrainCode: number | null,
): void {
  const { width, height, elevCm, classes } = state;
  if (x0 < 0 || x0 >= width || y0 < 0 || y0 >= height) return;
  if (x1 < 0 || x1 >= width || y1 < 0 || y1 >= height) return;
  const startElev = elevCm[y0 * width + x0] ?? 0;
  const endElev = elevCm[y1 * width + x1] ?? 0;
  const line = bresenhamCells(x0, y0, x1, y1);
  const n = Math.max(1, line.length - 1);
  for (let i = 0; i < line.length; i += 1) {
    const [lx, ly] = line[i]!;
    const t = i / n;
    const target = Math.round(startElev + (endElev - startElev) * t);
    const pts = brushFootprint(lx, ly, radius, "circle", width, height);
    for (const p of pts) {
      const idx = p.y * width + p.x;
      elevCm[idx] = target;
      if (paintTerrainCode !== null) {
        classes[idx] = paintTerrainCode;
      }
    }
  }
  enforceLocalGradeLimit(
    state,
    Math.min(x0, x1) - radius,
    Math.min(y0, y1) - radius,
    Math.max(x0, x1) + radius,
    Math.max(y0, y1) + radius,
  );
}

/** Paint a filled or hollow rectangle between `(x0, y0)` and `(x1, y1)`. */
export function applyRectStroke(
  state: EditorMapState,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  terrainCode: number,
  hollow: boolean,
): void {
  const { width, height, classes } = state;
  const minX = Math.max(0, Math.min(x0, x1));
  const maxX = Math.min(width - 1, Math.max(x0, x1));
  const minY = Math.max(0, Math.min(y0, y1));
  const maxY = Math.min(height - 1, Math.max(y0, y1));
  for (let y = minY; y <= maxY; y += 1) {
    for (let x = minX; x <= maxX; x += 1) {
      if (hollow && x > minX && x < maxX && y > minY && y < maxY) continue;
      classes[y * width + x] = terrainCode;
    }
  }
}

/** Contiguous 4-connected flood fill starting at `(sx, sy)`. */
export function applyFloodFill(
  state: EditorMapState,
  sx: number,
  sy: number,
  terrainCode: number,
): void {
  const { width, height, classes } = state;
  if (sx < 0 || sx >= width || sy < 0 || sy >= height) return;
  const startIdx = sy * width + sx;
  const targetCode = classes[startIdx];
  if (targetCode === undefined || targetCode === terrainCode) return;

  const visited = new Uint8Array(width * height);
  const queue: number[] = [startIdx];
  visited[startIdx] = 1;

  while (queue.length > 0) {
    const idx = queue.pop()!;
    classes[idx] = terrainCode;
    const x = idx % width;
    const y = Math.floor(idx / width);
    const neighbors: [number, number][] = [
      [x - 1, y],
      [x + 1, y],
      [x, y - 1],
      [x, y + 1],
    ];
    for (const [nx, ny] of neighbors) {
      if (nx < 0 || nx >= width || ny < 0 || ny >= height) continue;
      const nIdx = ny * width + nx;
      if (visited[nIdx] === 0 && classes[nIdx] === targetCode) {
        visited[nIdx] = 1;
        queue.push(nIdx);
      }
    }
  }
}

export type PresetTemplateId =
  | "proving_ground"
  | "mars_canyon"
  | "lunar_crater"
  | "ice_glacier"
  | "urban_compound"
  | "blank";

/** Generate a complete playable map from a named preset and seed. */
export function generatePresetMap(
  preset: PresetTemplateId,
  width = 64,
  height = 64,
  seed = 42,
): EditorMapState {
  const w = Math.max(16, Math.min(256, Math.trunc(width)));
  const h = Math.max(16, Math.min(256, Math.trunc(height)));
  const classes = new Uint8Array(w * h);
  const elevCm = new Int16Array(w * h);

  let biome: BiomeId = "temperate";
  let label = "Custom Proving Ground";
  let id = "custom_arena";

  if (preset === "mars_canyon") {
    biome = "mars";
    label = "Ares Canyon Shelf";
    id = "ares_canyon";
    classes.fill(8); // sand
    const midX = Math.floor(w / 2);
    const canyonHalf = Math.max(6, Math.floor(w / 5));
    for (let y = 0; y < h; y += 1) {
      const meander = Math.round(Math.sin((y + seed) * 0.12) * 4);
      for (let x = 0; x < w; x += 1) {
        const idx = y * w + x;
        const dist = Math.abs(x - (midX + meander));
        if (dist <= 2) {
          classes[idx] = 2; // dry channel ground
          elevCm[idx] = -60;
        } else if (dist > canyonHalf) {
          const rim = Math.min(8, dist - canyonHalf);
          elevCm[idx] = rim * 40;
          if (rim >= 5) {
            classes[idx] = hash2d(x, y, seed) < 220 ? 9 : 17; // rock / regolith mesa top
          }
        } else if (hash2d(x, y, seed + 3) < 90) {
          classes[idx] = 6; // rubble scatter
        }
      }
    }
  } else if (preset === "lunar_crater") {
    biome = "lunar";
    label = "Tycho Crater Basin";
    id = "tycho_basin";
    classes.fill(17); // regolith
    const cx = Math.floor(w / 2);
    const cy = Math.floor(h / 2);
    const rimR = Math.max(10, Math.floor(Math.min(w, h) * 0.32));
    for (let y = 0; y < h; y += 1) {
      for (let x = 0; x < w; x += 1) {
        const idx = y * w + x;
        const d = Math.hypot(x - cx, y - cy);
        if (Math.abs(d - rimR) <= 4) {
          elevCm[idx] = Math.round((4 - Math.abs(d - rimR)) * 42);
          classes[idx] = 9; // rock rim
        } else if (d < rimR - 4) {
          elevCm[idx] = -Math.round(Math.min(4, (rimR - 4 - d) * 0.35) * 35);
          if (d < 3.5) {
            elevCm[idx] = 90; // central peak
            classes[idx] = 9;
          }
        } else if (hash2d(x, y, seed) < 80) {
          classes[idx] = 6; // ejecta rubble
        }
      }
    }
  } else if (preset === "ice_glacier") {
    biome = "ice";
    label = "Europa Glacial Shelf";
    id = "europa_shelf";
    classes.fill(16); // ice
    const midX = Math.floor(w / 2);
    for (let y = 0; y < h; y += 1) {
      const leadX = midX + Math.round(Math.cos((y + seed) * 0.1) * 3);
      for (let x = 0; x < w; x += 1) {
        const idx = y * w + x;
        const d = Math.abs(x - leadX);
        if (d <= 1 && Math.abs(y - Math.floor(h / 2)) > 4) {
          classes[idx] = 15; // deep meltwater lead, leaving an ice bridge in the center
        } else if ((x + y + seed) % 13 < 3) {
          elevCm[idx] = 95;
          classes[idx] = 9; // pressure ridge
        } else if (hash2d(x, y, seed) < 180) {
          classes[idx] = 3; // snow drift (grass glyph mapped to snow in ice biome)
        }
      }
    }
  } else if (preset === "urban_compound") {
    biome = "temperate";
    label = "Forward Operating Compound";
    id = "forward_compound";
    classes.fill(3); // grass
    const cx = Math.floor(w / 2);
    const cy = Math.floor(h / 2);
    // East-west & north-south roads
    for (let x = 0; x < w; x += 1) {
      classes[cy * w + x] = 7;
      if (cy + 1 < h) classes[(cy + 1) * w + x] = 7;
    }
    for (let y = 0; y < h; y += 1) {
      classes[y * w + cx] = 7;
    }
    // Central compound walls + doors + paved courtyard
    const bx0 = Math.max(4, cx - 10);
    const bx1 = Math.min(w - 5, cx + 10);
    const by0 = Math.max(4, cy - 8);
    const by1 = Math.min(h - 5, cy + 8);
    for (let y = by0; y <= by1; y += 1) {
      for (let x = bx0; x <= bx1; x += 1) {
        const idx = y * w + x;
        const onEdge = x === bx0 || x === bx1 || y === by0 || y === by1;
        const isGate =
          (x === cx && (y === by0 || y === by1)) ||
          ((y === cy || y === cy + 1) && (x === bx0 || x === bx1));
        if (isGate) {
          classes[idx] = 11; // door
        } else if (onEdge) {
          classes[idx] = 10; // wall
        } else {
          classes[idx] = 1; // paved courtyard
          elevCm[idx] = 30;
        }
      }
    }
  } else if (preset === "blank") {
    biome = "temperate";
    label = "Blank Sector";
    id = "blank_sector";
    classes.fill(3); // grass
  } else {
    // "proving_ground"
    biome = "temperate";
    label = "The Proving Ground";
    id = "proving_ground";
    classes.fill(3); // grass
    const midX = Math.floor(w / 2);
    const midY = Math.floor(h / 2);
    for (let y = 0; y < h; y += 1) {
      const riverX = midX + Math.round(Math.sin((y + seed) * 0.11) * 2);
      for (let x = 0; x < w; x += 1) {
        const idx = y * w + x;
        if (Math.abs(y - midY) <= 1) {
          classes[idx] = 7; // road across the ford
        } else if (Math.abs(x - riverX) <= 1) {
          classes[idx] = Math.abs(x - riverX) === 0 ? 15 : 4; // water
          elevCm[idx] = -30;
        } else {
          // Gentle rolling hills on the flanks
          const hill = Math.hypot(x - Math.floor(w * 0.72), y - Math.floor(h * 0.68));
          if (hill < 10) {
            elevCm[idx] = Math.round((10 - hill) * 28);
            if (hill < 5) classes[idx] = 14; // heavy woods on hill
            else if (hill < 8) classes[idx] = 13; // light woods
          } else if (hash2d(x, y, seed) < 85) {
            classes[idx] = 13; // scattered light woods
          } else if (hash2d(x, y, seed + 9) < 55) {
            classes[idx] = 2; // dirt patch
          }
        }
      }
    }
  }

  enforceLocalGradeLimit(
    {
      id,
      label,
      note: "Authored in the web Terrain & Arena Studio.",
      biome,
      width: w,
      height: h,
      cellCm: 1000,
      classes,
      elevCm,
      redMuster: { rect: [2, Math.floor(h / 2) - 4, 8, Math.floor(h / 2) + 4], headingDeg: 90 },
      blueMuster: {
        rect: [w - 9, Math.floor(h / 2) - 4, w - 3, Math.floor(h / 2) + 4],
        headingDeg: 270,
      },
      spots: [],
      camera: { x: Math.max(4, Math.floor(w * 0.22)), y: Math.floor(h / 2), headingDeg: 90 },
    },
    0,
    0,
    w - 1,
    h - 1,
  );

  return {
    id,
    label,
    note: "Authored in the web Terrain & Arena Studio.",
    biome,
    width: w,
    height: h,
    cellCm: 1000,
    classes,
    elevCm,
    redMuster: { rect: [2, Math.floor(h / 2) - 4, 8, Math.floor(h / 2) + 4], headingDeg: 90 },
    blueMuster: {
      rect: [w - 9, Math.floor(h / 2) - 4, w - 3, Math.floor(h / 2) + 4],
      headingDeg: 270,
    },
    spots: [
      { letter: "A", name: "Center Crossing", cell: [Math.floor(w / 2), Math.floor(h / 2)] },
      {
        letter: "B",
        name: "North Overlook",
        cell: [Math.floor(w * 0.68), Math.floor(h * 0.72)],
      },
    ],
    camera: { x: Math.max(4, Math.floor(w * 0.22)), y: Math.floor(h / 2), headingDeg: 90 },
  };
}

const B64_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function bytesToBase64(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 3) {
    const b0 = bytes[i] ?? 0;
    const has1 = i + 1 < bytes.length;
    const has2 = i + 2 < bytes.length;
    const b1 = has1 ? (bytes[i + 1] ?? 0) : 0;
    const b2 = has2 ? (bytes[i + 2] ?? 0) : 0;
    out += B64_CHARS[b0 >> 2];
    out += B64_CHARS[((b0 & 0x03) << 4) | (b1 >> 4)];
    out += has1 ? B64_CHARS[((b1 & 0x0f) << 2) | (b2 >> 6)] : "=";
    out += has2 ? B64_CHARS[b2 & 0x3f] : "=";
  }
  return out;
}

function base64ToBytes(text: string): Uint8Array | null {
  const clean = text.replace(/\s+/g, "");
  if (clean.length === 0 || clean.length % 4 !== 0) return null;
  const out: number[] = [];
  for (let i = 0; i < clean.length; i += 4) {
    const c0 = B64_CHARS.indexOf(clean[i]!);
    const c1 = B64_CHARS.indexOf(clean[i + 1]!);
    const c2 = clean[i + 2] === "=" ? -1 : B64_CHARS.indexOf(clean[i + 2]!);
    const c3 = clean[i + 3] === "=" ? -1 : B64_CHARS.indexOf(clean[i + 3]!);
    if (c0 < 0 || c1 < 0 || (clean[i + 2] !== "=" && c2 < 0) || (clean[i + 3] !== "=" && c3 < 0)) {
      return null;
    }
    out.push((c0 << 2) | (c1 >> 4));
    if (c2 >= 0) {
      out.push(((c1 & 0x0f) << 4) | (c2 >> 2));
      if (c3 >= 0) {
        out.push(((c2 & 0x03) << 6) | c3);
      }
    }
  }
  return new Uint8Array(out);
}

/**
 * Encode `EditorMapState` cells into the exact `mush_atlas::Cells::encode`
 * base64 format (`[w u16 LE][h u16 LE]` + `[count u16 LE][class u8][elev_cm i16 LE][cover u8][room u16 LE]`).
 */
export function encodeTileCellsBase64(state: EditorMapState): string {
  const total = state.width * state.height;
  const bytes: number[] = [
    state.width & 0xff,
    (state.width >> 8) & 0xff,
    state.height & 0xff,
    (state.height >> 8) & 0xff,
  ];
  let runClass = state.classes[0] ?? 0;
  let runElev = state.elevCm[0] ?? 0;
  let runCount = 0;

  const flush = (count: number, cls: number, elev: number): void => {
    if (count <= 0) return;
    bytes.push(count & 0xff, (count >> 8) & 0xff);
    bytes.push(cls & 0xff);
    const e16 = elev & 0xffff;
    bytes.push(e16 & 0xff, (e16 >> 8) & 0xff);
    bytes.push(0); // cover = 0
    bytes.push(0, 0); // room = 0
  };

  for (let i = 0; i < total; i += 1) {
    const cls = state.classes[i] ?? 0;
    const elev = state.elevCm[i] ?? 0;
    if (cls === runClass && elev === runElev && runCount < 0xffff) {
      runCount += 1;
    } else {
      flush(runCount, runClass, runElev);
      runClass = cls;
      runElev = elev;
      runCount = 1;
    }
  }
  flush(runCount, runClass, runElev);
  return bytesToBase64(new Uint8Array(bytes));
}

/**
 * Decode a `mush_atlas::Cells::encode` base64 string into `{ width, height, classes, elevCm }`.
 */
export function decodeTileCellsBase64(text: string): {
  width: number;
  height: number;
  classes: Uint8Array;
  elevCm: Int16Array;
} | null {
  const bytes = base64ToBytes(text);
  if (!bytes || bytes.length < 4) return null;
  const width = (bytes[0] ?? 0) | ((bytes[1] ?? 0) << 8);
  const height = (bytes[2] ?? 0) | ((bytes[3] ?? 0) << 8);
  if (width <= 0 || height <= 0 || width > 256 || height > 256) return null;
  const total = width * height;
  if ((bytes.length - 4) % 8 !== 0) return null;

  const classes = new Uint8Array(total);
  const elevCm = new Int16Array(total);
  let pos = 0;
  for (let off = 4; off < bytes.length; off += 8) {
    const count = (bytes[off] ?? 0) | ((bytes[off + 1] ?? 0) << 8);
    const cls = bytes[off + 2] ?? 0;
    const eRaw = (bytes[off + 3] ?? 0) | ((bytes[off + 4] ?? 0) << 8);
    const elev = eRaw >= 0x8000 ? eRaw - 0x10000 : eRaw;
    if (count <= 0 || pos + count > total) return null;
    for (let i = 0; i < count; i += 1) {
      classes[pos] = cls;
      elevCm[pos] = elev;
      pos += 1;
    }
  }
  if (pos !== total) return null;
  return { width, height, classes, elevCm };
}

/** Export the map state as a complete `tacsim` `ArenaDef` JSON object. */
export function exportArenaDefJson(state: EditorMapState): Record<string, unknown> {
  return {
    label: state.label,
    note: state.note,
    cols: state.width,
    rows: state.height,
    biome: state.biome,
    procedural_relief: false,
    tile: encodeTileCellsBase64(state),
    red_muster: state.redMuster.rect,
    blue_muster: state.blueMuster.rect,
    red_heading_mdeg: state.redMuster.headingDeg * 1000,
    blue_heading_mdeg: state.blueMuster.headingDeg * 1000,
    spots: state.spots.map((s) => ({
      letter: s.letter,
      name: s.name,
      cell: s.cell,
    })),
  };
}

/** Import an `ArenaDef` JSON object into an `EditorMapState`. */
export function importArenaDefJson(
  raw: unknown,
  fallbackId = "imported_arena",
): EditorMapState | null {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return null;
  const obj = raw as Record<string, unknown>;
  if (typeof obj.tile !== "string") return null;
  const decoded = decodeTileCellsBase64(obj.tile);
  if (!decoded) return null;
  const { width, height, classes, elevCm } = decoded;
  const biomeRaw = typeof obj.biome === "string" ? obj.biome.toLowerCase() : "temperate";
  const biome: BiomeId =
    biomeRaw === "mars" || biomeRaw === "lunar" || biomeRaw === "ice" ? biomeRaw : "temperate";
  const parseRect = (v: unknown, def: [number, number, number, number]): [number, number, number, number] =>
    Array.isArray(v) &&
    v.length === 4 &&
    v.every((n) => typeof n === "number" && Number.isFinite(n))
      ? [Math.trunc(v[0]), Math.trunc(v[1]), Math.trunc(v[2]), Math.trunc(v[3])]
      : def;
  return {
    id: fallbackId,
    label: typeof obj.label === "string" && obj.label.trim() ? obj.label.trim() : "Imported Arena",
    note: typeof obj.note === "string" ? obj.note : "",
    biome,
    width,
    height,
    cellCm: 1000,
    classes,
    elevCm,
    redMuster: {
      rect: parseRect(obj.red_muster, [2, 2, 6, 6]),
      headingDeg:
        typeof obj.red_heading_mdeg === "number" ? Math.round(obj.red_heading_mdeg / 1000) : 90,
    },
    blueMuster: {
      rect: parseRect(obj.blue_muster, [width - 7, height - 7, width - 3, height - 3]),
      headingDeg:
        typeof obj.blue_heading_mdeg === "number" ? Math.round(obj.blue_heading_mdeg / 1000) : 270,
    },
    spots: Array.isArray(obj.spots)
      ? obj.spots.flatMap((s): ArenaSpot[] => {
          if (typeof s !== "object" || s === null) return [];
          const rec = s as Record<string, unknown>;
          if (
            typeof rec.letter === "string" &&
            rec.letter.length === 1 &&
            typeof rec.name === "string" &&
            Array.isArray(rec.cell) &&
            typeof rec.cell[0] === "number" &&
            typeof rec.cell[1] === "number"
          ) {
            return [
              {
                letter: rec.letter,
                name: rec.name,
                cell: [Math.trunc(rec.cell[0]), Math.trunc(rec.cell[1])],
              },
            ];
          }
          return [];
        })
      : [],
    camera: {
      x: Math.floor(width / 3),
      y: Math.floor(height / 2),
      headingDeg: 90,
    },
  };
}

/**
 * Synthesize `arena.map` and `arena.cockpit` wire feeds from the editor state
 * so `mush-pixel-wasm` renders a live 1st-person, 2.5D, or overhead preview
 * from the editor's camera pin (`state.camera`), complete with sample units at
 * the muster zones so builders can preview scale, sightlines, and silhouettes!
 */
export function editorToPictureFeeds(state: EditorMapState): {
  map: Record<string, unknown>;
  cockpit: Record<string, unknown>;
} {
  const { width, height, classes, elevCm, biome, camera } = state;
  const rows: [string, number][][] = [];
  const elevField: string[] = [];
  const usedCodes = new Set<number>();

  for (let y = 0; y < height; y += 1) {
    const rowRuns: [string, number][] = [];
    let curGlyph = "";
    let curCount = 0;
    let elevLine = "";
    for (let x = 0; x < width; x += 1) {
      const idx = y * width + x;
      const code = classes[idx] ?? 0;
      usedCodes.add(code);
      const glyph = terrainByCode(code).glyph;
      if (glyph === curGlyph && curCount > 0) {
        curCount += 1;
      } else {
        if (curCount > 0) rowRuns.push([curGlyph, curCount]);
        curGlyph = glyph;
        curCount = 1;
      }
      const dm = Math.max(-120, Math.min(120, Math.round((elevCm[idx] ?? 0) / 10)));
      elevLine += String.fromCharCode(dm + 120 + 33);
    }
    if (curCount > 0) rowRuns.push([curGlyph, curCount]);
    rows.push(rowRuns);
    elevField.push(elevLine);
  }

  const legend: [string, string][] = [...usedCodes]
    .sort((a, b) => a - b)
    .map((code) => {
      const meta = terrainByCode(code);
      return [meta.glyph, meta.id];
    });

  const redCx = Math.round((state.redMuster.rect[0] + state.redMuster.rect[2]) / 2);
  const redCy = Math.round((state.redMuster.rect[1] + state.redMuster.rect[3]) / 2);
  const blueCx = Math.round((state.blueMuster.rect[0] + state.blueMuster.rect[2]) / 2);
  const blueCy = Math.round((state.blueMuster.rect[1] + state.blueMuster.rect[3]) / 2);

  const sampleContacts = [
    {
      designator: "R1",
      class: "biped",
      cell: [redCx, redCy],
      friendly: true,
      heading_deg: state.redMuster.headingDeg,
    },
    {
      designator: "R2",
      class: "scout",
      cell: [Math.min(width - 1, redCx + 2), Math.max(0, redCy - 2)],
      friendly: true,
      heading_deg: state.redMuster.headingDeg,
    },
    {
      designator: "B1",
      class: "heavy",
      cell: [blueCx, blueCy],
      friendly: false,
      heading_deg: state.blueMuster.headingDeg,
    },
    {
      designator: "B2",
      class: "tank",
      cell: [Math.max(0, blueCx - 2), Math.min(height - 1, blueCy + 2)],
      friendly: false,
      heading_deg: state.blueMuster.headingDeg,
    },
  ].map((c) => {
    const dx = c.cell[0]! - camera.x;
    const dy = c.cell[1]! - camera.y;
    const distM = Math.round(Math.hypot(dx, dy) * (state.cellCm / 100));
    const bearing = Math.round(((Math.atan2(dx, dy) * 180) / Math.PI + 360) % 360);
    const rel = ((bearing - camera.headingDeg + 540) % 360) - 180;
    return {
      designator: c.designator,
      alias: null,
      class: c.class,
      bearing_deg: bearing,
      relative_deg: rel,
      band: distM < 180 ? "near" : distM < 420 ? "mid" : "far",
      range_m: distM,
      age_ms: 0,
      age_s: 0,
      moving: false,
      cell: c.cell,
      friendly: c.friendly,
      condition: null,
      aspect: "front",
      reads_dead: false,
      odds: [],
      fatal: null,
    };
  });

  const surfaceName = `${state.id}_${biome}`;

  return {
    map: {
      schema: { id: "arena.map", version: 1, retain: "latest" },
      seated: true,
      match: 1,
      surface: surfaceName,
      biome,
      cell_cm: state.cellCm,
      width,
      height,
      origin_cell: [0, 0],
      rows,
      legend,
      spots: state.spots,
      elev_field: elevField,
    },
    cockpit: {
      schema: { id: "arena.cockpit", version: 3, retain: "latest" },
      seated: true,
      match: 1,
      callsign: "CAM",
      unit_type: "biped",
      cell: [camera.x, camera.y],
      power: "online",
      blocked: false,
      wreck: false,
      immobile: false,
      speed_kph: 0,
      heading_deg: camera.headingDeg,
      heat: { heat: 0, max: 100, overheated: false },
      receiver: null,
      ecm: null,
      tempo: "calm",
      mode: "skirmish",
      scope: "clear",
      lock_designator: "B1",
      lock_range_m: sampleContacts[2]?.range_m ?? 200,
      lock_percent: 75,
      sections: [],
      weapons: [],
      rules: [],
      contacts: sampleContacts,
      scan_tables: [],
      notices: [],
      catchup: null,
      t_ms: 0,
      order: null,
      armed: true,
      weapons_free: true,
    },
  };
}
