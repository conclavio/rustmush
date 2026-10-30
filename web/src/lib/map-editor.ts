/**
 * Web Terrain & Arena Map Studio engine (doc 30 §8, D-30-8; cut WG.4).
 *
 * Implements:
 * - `ground.draft.v1` import/export (`exportGroundDraftJson`, `importGroundDraftJson`)
 * - 3-byte class RLE (`[cols u16 LE][rows u16 LE] + [count u16 LE][class u8]`), `le16`
 *   elevation codec, and `zigzag-leb128-delta` wire elevation encoder (`D-30-2`, `D-30-6`)
 * - Multi-cell brush masks, Bresenham line strokes, contiguous flood fill, and
 *   optional "relax to passable" sculpt clamping (`D-30-4`, `D-30-8`)
 * - Passability overlay per drive (`tracked`, `wheeled`, `legged`) from `locomotion.json`
 * - Waypoint graph editing and auto-routing (`routeEditorMap`) matching `tacsim::ground::route`
 * - Viewshed overlay (`computeViewshed`) matching `tacsim::ground::viewshed` / `horizon.rs`
 * - Live §5 check report (`checkEditorMap`) matching `tacsim::ground::check`
 * - Interactive `relaxEditorSlopes` and `despeckleEditorMap` tools with changed-cell tracking
 */

import locomotionData from './fixtures/locomotion.json';

export type BiomeName = 'temperate' | 'mars' | 'lunar' | 'ice';

export const BIOME_OPTIONS: readonly BiomeName[] = [
  'temperate',
  'mars',
  'lunar',
  'ice'
] as const;

/**
 * Exact `mush_atlas::TerrainClass` byte codes (`0..=17`, `crates/mush-atlas/src/terrain.rs`).
 */
export type TerrainClassCode =
  | 0 // void
  | 1 // paved
  | 2 // ground
  | 3 // grass
  | 4 // water
  | 5 // floor
  | 6 // rubble
  | 7 // road
  | 8 // sand
  | 9 // rock
  | 10 // wall
  | 11 // door
  | 12 // platform
  | 13 // woods_light
  | 14 // woods_heavy
  | 15 // water_deep
  | 16 // ice
  | 17; // regolith

export interface TerrainSwatch {
  code: TerrainClassCode;
  word: string;
  glyph: string;
  label: string;
  color: string;
  shortcut?: string;
}

export const TERRAIN_PALETTE: readonly TerrainSwatch[] = [
  { code: 2, word: 'ground', glyph: ',', label: 'Ground', color: '#6b6552', shortcut: '1' },
  { code: 3, word: 'grass', glyph: '"', label: 'Grass', color: '#4a6b3b', shortcut: '2' },
  { code: 1, word: 'paved', glyph: '.', label: 'Paved', color: '#686c73', shortcut: '3' },
  { code: 7, word: 'road', glyph: '=', label: 'Road', color: '#827864', shortcut: '4' },
  { code: 8, word: 'sand', glyph: ':', label: 'Sand', color: '#a89060', shortcut: '5' },
  { code: 17, word: 'regolith', glyph: 'r', label: 'Regolith', color: '#786e68', shortcut: '6' },
  { code: 9, word: 'rock', glyph: '^', label: 'Rock (Surface)', color: '#5c5852', shortcut: '7' },
  { code: 6, word: 'rubble', glyph: 'x', label: 'Rubble', color: '#70685c', shortcut: '8' },
  { code: 10, word: 'wall', glyph: '#', label: 'Wall', color: '#9c968c', shortcut: '9' },
  { code: 13, word: 'woods_light', glyph: 't', label: 'Light Woods', color: '#385830', shortcut: '0' },
  { code: 14, word: 'woods_heavy', glyph: 'T', label: 'Heavy Woods', color: '#243e20' },
  { code: 4, word: 'water', glyph: '~', label: 'Shallow Water', color: '#2b5878' },
  { code: 15, word: 'water_deep', glyph: 'W', label: 'Deep Water', color: '#1b3854' },
  { code: 16, word: 'ice', glyph: 'i', label: 'Ice', color: '#8ab4cc' },
  { code: 5, word: 'floor', glyph: '_', label: 'Floor', color: '#585c64' },
  { code: 11, word: 'door', glyph: '+', label: 'Door', color: '#b48a48' },
  { code: 12, word: 'platform', glyph: 'D', label: 'Platform', color: '#7a828c' },
  { code: 0, word: 'void', glyph: ' ', label: 'Void', color: '#14161a' }
] as const;

const CLASS_META: Record<number, { glyph: string; word: string }> = {
  0: { glyph: ' ', word: 'void' },
  1: { glyph: '.', word: 'paved' },
  2: { glyph: ',', word: 'ground' },
  3: { glyph: '"', word: 'grass' },
  4: { glyph: '~', word: 'water' },
  5: { glyph: '_', word: 'floor' },
  6: { glyph: 'x', word: 'rubble' },
  7: { glyph: '=', word: 'road' },
  8: { glyph: ':', word: 'sand' },
  9: { glyph: '^', word: 'rock' },
  10: { glyph: '#', word: 'wall' },
  11: { glyph: '+', word: 'door' },
  12: { glyph: 'D', word: 'platform' },
  13: { glyph: 't', word: 'woods_light' },
  14: { glyph: 'T', word: 'woods_heavy' },
  15: { glyph: 'W', word: 'water_deep' },
  16: { glyph: 'i', word: 'ice' },
  17: { glyph: 'r', word: 'regolith' }
};

export type EditorTool =
  | 'paint'
  | 'line'
  | 'fill'
  | 'sculpt'
  | 'spot'
  | 'waypoint'
  | 'zone_red'
  | 'zone_blue'
  | 'camera_pin';

export type SculptMode = 'raise' | 'lower' | 'flatten' | 'smooth';
export type BrushShape = 'circle' | 'square';
export type DriveName = 'tracked' | 'wheeled' | 'legged';
export type StudioOverlayMode = 'none' | 'passability' | 'viewshed';

export interface SculptBrush {
  mode: SculptMode;
  shape: BrushShape;
  radius: number;
  strengthCm: number;
  targetElevCm: number;
  falloff: number;
  relaxToPassable?: boolean;
}

export interface EditorKeySpot {
  name: string;
  x: number;
  y: number;
}

export interface EditorEntryZone {
  team: 'red' | 'blue';
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

export interface EditorWaypointNode {
  id: string;
  x: number;
  y: number;
}

export interface EditorWaypointGraph {
  nodes: EditorWaypointNode[];
  edges: [string, string][];
}

export interface EditorArenaMode {
  mode: 'hold';
  spot: string;
  radius_cells: number;
  hold_ms: number;
}

export interface EditorTankDef {
  id: string;
  label: string;
  unit: string;
  team: 'red' | 'blue';
  slot: number;
  x: number;
  y: number;
  heading_deg: number;
}

export interface EditorProvenance {
  source: 'generator' | 'extractor' | 'studio';
  version: string;
  seed?: number;
  inputs?: string[];
}

export interface EditorCameraPin {
  x: number;
  y: number;
  headingDeg: number;
}

export interface EditorMapState {
  id: string;
  label: string;
  biome: BiomeName;
  body: string;
  width: number;
  height: number;
  cellCm: number;
  originM: [number, number];
  classes: Uint8Array;
  elevCm: Int16Array;
  keySpots: EditorKeySpot[];
  entryZones: EditorEntryZone[];
  waypoints: EditorWaypointGraph;
  modes: EditorArenaMode[];
  tanks: EditorTankDef[];
  atlasVersion: number;
  arenaVersion: number;
  provenance: EditorProvenance;
  note: string;
}

/**
 * The `ground.draft.v1` JSON schema (doc 30 §4, D-30-2).
 */
export const GROUND_DRAFT_SCHEMA = 'ground.draft.v1';

export interface GroundDraftJson {
  schema: string;
  id: string;
  label: string;
  biome: BiomeName;
  body: string;
  origin_m: [number, number];
  cell_cm: number;
  cols: number;
  rows: number;
  classes: string;
  elev: string;
  entry_zones: EditorEntryZone[];
  key_spots: EditorKeySpot[];
  waypoints: EditorWaypointGraph;
  modes?: EditorArenaMode[];
  tanks?: EditorTankDef[];
  atlas_version?: number;
  arena_version?: number;
  provenance: EditorProvenance;
  note?: string;
}

/**
 * The §5 grade and passability check report (doc 30 §5 & §8, matching `tacsim::ground::CheckReport`).
 */
export interface GroundCheckReport {
  id: string;
  cols: number;
  rows: number;
  quadrants: number;
  map_bytes: number;
  map_max_bytes: number;
  tracked_passable_pct: number;
  wheeled_passable_pct: number;
  legged_passable_pct: number;
  zones_reachable_tracked: boolean;
  zones_reachable_wheeled: boolean;
  zones_reachable_legged: boolean;
  key_spots_wheeled_reachable: boolean;
  unreachable_key_spots: string[];
  waypoints_reachable: boolean;
  unreachable_waypoints: string[];
  max_grade_pct: number;
  max_road_grade_pct: number;
  max_entry_zone_step_cm: number;
  passed: boolean;
  failures: string[];
}

/**
 * Elevation bounds (`i16` cell range, `±327.67 m`) and studio slider defaults (`±150 m`).
 */
export const MIN_ELEV_CM = -32768;
export const MAX_ELEV_CM = 32767;
export const DEFAULT_SLIDER_MIN_ELEV_CM = -15000;
export const DEFAULT_SLIDER_MAX_ELEV_CM = 15000;
export const MAX_ORTHOGONAL_STEP_CM = 450;
export const MAX_DIAGONAL_STEP_CM = 630;
export const MAX_MUSTER_STEP_CM = 50;
export const MAP_MAX_BYTES = 262_144;
export const EYE_HEIGHT_CM = 150;
export const DEFAULT_HULL_SIZE_CM = 300;

export function createEditorMap(
  width = 256,
  height = 256,
  biome: BiomeName = 'temperate',
  fillClass: TerrainClassCode = 3
): EditorMapState {
  const w = Math.max(16, Math.min(512, Math.floor(width)));
  const h = Math.max(16, Math.min(512, Math.floor(height)));
  const classes = new Uint8Array(w * h);
  classes.fill(fillClass);
  const elevCm = new Int16Array(w * h);
  const marginX = Math.max(2, Math.floor(w * 0.06));
  const marginY = Math.max(2, Math.floor(h * 0.25));
  const zoneW = Math.max(3, Math.floor(w * 0.08));
  const zoneH = Math.max(4, Math.floor(h * 0.5));
  return {
    id: 'custom_arena',
    label: 'Custom Arena',
    biome,
    body: 'luna',
    width: w,
    height: h,
    cellCm: 1000,
    originM: [0, 0],
    classes,
    elevCm,
    keySpots: [
      { name: 'The Ford', x: Math.floor(w * 0.35), y: Math.floor(h * 0.45) },
      { name: 'Signal Hill', x: Math.floor(w * 0.5), y: Math.floor(h * 0.55) },
      { name: 'Crossroads', x: Math.floor(w * 0.65), y: Math.floor(h * 0.45) }
    ],
    entryZones: [
      {
        team: 'red',
        x0: marginX,
        y0: marginY,
        x1: Math.min(w - 1, marginX + zoneW),
        y1: Math.min(h - 1, marginY + zoneH)
      },
      {
        team: 'blue',
        x0: Math.max(0, w - 1 - marginX - zoneW),
        y0: marginY,
        x1: Math.max(0, w - 1 - marginX),
        y1: Math.min(h - 1, marginY + zoneH)
      }
    ],
    waypoints: { nodes: [], edges: [] },
    modes: [],
    tanks: [],
    atlasVersion: 1,
    arenaVersion: 1,
    provenance: {
      source: 'studio',
      version: 'v1'
    },
    note: ''
  };
}

export function cloneEditorMap(state: EditorMapState): EditorMapState {
  return {
    id: state.id,
    label: state.label,
    biome: state.biome,
    body: state.body,
    width: state.width,
    height: state.height,
    cellCm: state.cellCm,
    originM: [state.originM[0], state.originM[1]],
    classes: new Uint8Array(state.classes),
    elevCm: new Int16Array(state.elevCm),
    keySpots: state.keySpots.map((s) => ({ ...s })),
    entryZones: state.entryZones.map((z) => ({ ...z })),
    waypoints: {
      nodes: state.waypoints.nodes.map((n) => ({ ...n })),
      edges: state.waypoints.edges.map((e) => [e[0], e[1]])
    },
    modes: state.modes.map((m) => ({ ...m })),
    tanks: state.tanks.map((t) => ({ ...t })),
    atlasVersion: state.atlasVersion,
    arenaVersion: state.arenaVersion,
    provenance: {
      source: state.provenance.source,
      version: state.provenance.version,
      ...(state.provenance.seed !== undefined ? { seed: state.provenance.seed } : {}),
      ...(state.provenance.inputs ? { inputs: [...state.provenance.inputs] } : {})
    },
    note: state.note
  };
}

export class EditorUndoStack {
  private undoStack: EditorMapState[] = [];
  private redoStack: EditorMapState[] = [];
  private readonly maxDepth: number;

  constructor(maxDepth = 50) {
    this.maxDepth = maxDepth;
  }

  push(snapshot: EditorMapState): void {
    this.undoStack.push(cloneEditorMap(snapshot));
    if (this.undoStack.length > this.maxDepth) {
      this.undoStack.shift();
    }
    this.redoStack.length = 0;
  }

  undo(current: EditorMapState): EditorMapState | null {
    const prev = this.undoStack.pop();
    if (!prev) return null;
    this.redoStack.push(cloneEditorMap(current));
    return prev;
  }

  redo(current: EditorMapState): EditorMapState | null {
    const next = this.redoStack.pop();
    if (!next) return null;
    this.undoStack.push(cloneEditorMap(current));
    return next;
  }

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  clear(): void {
    this.undoStack.length = 0;
    this.redoStack.length = 0;
  }
}

export interface BrushSample {
  x: number;
  y: number;
  weight: number;
}

export function brushFootprint(
  width: number,
  height: number,
  cx: number,
  cy: number,
  radius: number,
  shape: BrushShape,
  falloff = 0.5
): BrushSample[] {
  const r = Math.max(0, Math.floor(radius));
  const samples: BrushSample[] = [];
  if (r === 0) {
    if (cx >= 0 && cx < width && cy >= 0 && cy < height) {
      samples.push({ x: cx, y: cy, weight: 1 });
    }
    return samples;
  }
  const rSq = (r + 0.35) * (r + 0.35);
  for (let dy = -r; dy <= r; dy++) {
    const y = cy + dy;
    if (y < 0 || y >= height) continue;
    for (let dx = -r; dx <= r; dx++) {
      const x = cx + dx;
      if (x < 0 || x >= width) continue;
      if (shape === 'circle') {
        const dSq = dx * dx + dy * dy;
        if (dSq > rSq) continue;
        const dist = Math.sqrt(dSq) / (r + 0.35);
        const w = falloff <= 0 ? 1 : Math.max(0.05, 1 - Math.pow(dist, 1 / Math.max(0.15, falloff)));
        samples.push({ x, y, weight: w });
      } else {
        const dist = Math.max(Math.abs(dx), Math.abs(dy)) / (r + 0.35);
        const w = falloff <= 0 ? 1 : Math.max(0.05, 1 - dist * falloff);
        samples.push({ x, y, weight: w });
      }
    }
  }
  return samples;
}

export function bresenhamLine(x0: number, y0: number, x1: number, y1: number): [number, number][] {
  const pts: [number, number][] = [];
  let cx = Math.floor(x0);
  let cy = Math.floor(y0);
  const tx = Math.floor(x1);
  const ty = Math.floor(y1);
  const dx = Math.abs(tx - cx);
  const sx = cx < tx ? 1 : -1;
  const dy = -Math.abs(ty - cy);
  const sy = cy < ty ? 1 : -1;
  let err = dx + dy;

  while (true) {
    pts.push([cx, cy]);
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
  return pts;
}

export function paintTerrainBrush(
  state: EditorMapState,
  cx: number,
  cy: number,
  radius: number,
  shape: BrushShape,
  terrainClass: TerrainClassCode
): boolean {
  const footprint = brushFootprint(state.width, state.height, cx, cy, radius, shape, 0);
  let changed = false;
  for (const pt of footprint) {
    const idx = pt.y * state.width + pt.x;
    if (state.classes[idx] !== terrainClass) {
      state.classes[idx] = terrainClass;
      changed = true;
    }
  }
  return changed;
}

export function paintTerrainLine(
  state: EditorMapState,
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  radius: number,
  shape: BrushShape,
  terrainClass: TerrainClassCode
): boolean {
  let changed = false;
  for (const [x, y] of bresenhamLine(x0, y0, x1, y1)) {
    if (paintTerrainBrush(state, x, y, radius, shape, terrainClass)) {
      changed = true;
    }
  }
  return changed;
}

export function floodFillTerrain(
  state: EditorMapState,
  startX: number,
  startY: number,
  replacement: TerrainClassCode
): number {
  if (startX < 0 || startX >= state.width || startY < 0 || startY >= state.height) {
    return 0;
  }
  const startIdx = startY * state.width + startX;
  const target = state.classes[startIdx];
  if (target === replacement) return 0;

  const visited = new Uint8Array(state.width * state.height);
  const queue: number[] = [startIdx];
  visited[startIdx] = 1;
  let filled = 0;

  while (queue.length > 0) {
    const idx = queue.pop()!;
    state.classes[idx] = replacement;
    filled++;
    const x = idx % state.width;
    const y = (idx - x) / state.width;

    const neighbors: [number, number][] = [
      [x - 1, y],
      [x + 1, y],
      [x, y - 1],
      [x, y + 1]
    ];
    for (const [nx, ny] of neighbors) {
      if (nx < 0 || nx >= state.width || ny < 0 || ny >= state.height) continue;
      const nIdx = ny * state.width + nx;
      if (visited[nIdx] === 0 && state.classes[nIdx] === target) {
        visited[nIdx] = 1;
        queue.push(nIdx);
      }
    }
  }
  return filled;
}

export function isInEntryZone(state: EditorMapState, x: number, y: number): boolean {
  for (const z of state.entryZones) {
    const x0 = Math.min(z.x0, z.x1);
    const x1 = Math.max(z.x0, z.x1);
    const y0 = Math.min(z.y0, z.y1);
    const y1 = Math.max(z.y0, z.y1);
    if (x >= x0 && x <= x1 && y >= y0 && y <= y1) {
      return true;
    }
  }
  return false;
}

export function clampElevationCm(val: number): number {
  return Math.max(MIN_ELEV_CM, Math.min(MAX_ELEV_CM, Math.round(val)));
}

/**
 * Optional local grade relaxation around `(cx, cy)` when `brush.relaxToPassable` is enabled.
 */
export function enforceLocalGradeLimit(
  state: EditorMapState,
  cx: number,
  cy: number,
  radius: number
): void {
  const w = state.width;
  const h = state.height;
  const minX = Math.max(0, cx - radius - 2);
  const maxX = Math.min(w - 1, cx + radius + 2);
  const minY = Math.max(0, cy - radius - 2);
  const maxY = Math.min(h - 1, cy + radius + 2);
  const orthCap = Math.trunc((state.cellCm * 45) / 100);
  const diagCap = Math.trunc((Math.trunc((state.cellCm * 141) / 100) * 45) / 100);

  for (let pass = 0; pass < 3; pass++) {
    for (let y = minY; y <= maxY; y++) {
      for (let x = minX; x <= maxX; x++) {
        const idx = y * w + x;
        const inMuster = isInEntryZone(state, x, y);
        const localOrthCap = inMuster ? MAX_MUSTER_STEP_CM : orthCap;
        const localDiagCap = inMuster ? MAX_MUSTER_STEP_CM : diagCap;
        let e = state.elevCm[idx] ?? 0;

        const orth: [number, number][] = [
          [x - 1, y],
          [x + 1, y],
          [x, y - 1],
          [x, y + 1]
        ];
        for (const [nx, ny] of orth) {
          if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
          const ne = state.elevCm[ny * w + nx] ?? 0;
          if (e - ne > localOrthCap) e = ne + localOrthCap;
          else if (ne - e > localOrthCap) e = ne - localOrthCap;
        }

        const diag: [number, number][] = [
          [x - 1, y - 1],
          [x + 1, y - 1],
          [x - 1, y + 1],
          [x + 1, y + 1]
        ];
        for (const [nx, ny] of diag) {
          if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
          const ne = state.elevCm[ny * w + nx] ?? 0;
          if (e - ne > localDiagCap) e = ne + localDiagCap;
          else if (ne - e > localDiagCap) e = ne - localDiagCap;
        }

        state.elevCm[idx] = clampElevationCm(e);
      }
    }
  }
}

export function applySculptBrush(
  state: EditorMapState,
  cx: number,
  cy: number,
  brush: SculptBrush
): boolean {
  const footprint = brushFootprint(
    state.width,
    state.height,
    cx,
    cy,
    brush.radius,
    brush.shape,
    brush.falloff
  );
  if (footprint.length === 0) return false;

  const prevSnapshot = brush.mode === 'smooth' ? new Int16Array(state.elevCm) : null;
  let changed = false;

  for (const sample of footprint) {
    const idx = sample.y * state.width + sample.x;
    const current = state.elevCm[idx] ?? 0;
    let next = current;

    if (brush.mode === 'raise') {
      next = clampElevationCm(current + brush.strengthCm * sample.weight);
    } else if (brush.mode === 'lower') {
      next = clampElevationCm(current - brush.strengthCm * sample.weight);
    } else if (brush.mode === 'flatten') {
      const target = clampElevationCm(brush.targetElevCm);
      next = clampElevationCm(current + (target - current) * sample.weight);
    } else if (brush.mode === 'smooth' && prevSnapshot) {
      let sum = 0;
      let count = 0;
      for (let dy = -1; dy <= 1; dy++) {
        const ny = sample.y + dy;
        if (ny < 0 || ny >= state.height) continue;
        for (let dx = -1; dx <= 1; dx++) {
          const nx = sample.x + dx;
          if (nx < 0 || nx >= state.width) continue;
          sum += prevSnapshot[ny * state.width + nx] ?? 0;
          count++;
        }
      }
      const avg = count > 0 ? sum / count : current;
      next = clampElevationCm(current + (avg - current) * sample.weight * 0.65);
    }

    if (next !== current) {
      state.elevCm[idx] = next;
      changed = true;
    }
  }

  if (changed && brush.relaxToPassable) {
    enforceLocalGradeLimit(state, cx, cy, brush.radius);
  }
  return changed;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

/**
 * Encode `classes` in `HeightfieldDef.classes` 3-byte run format (doc 30 §4, D-30-2):
 * `[cols u16 LE][rows u16 LE]` followed by runs of `[count u16 LE][class u8]`, base64-encoded.
 */
export function encodeClassesBase64(width: number, height: number, classes: Uint8Array): string {
  const total = width * height;
  const runs: [number, number][] = [];
  if (total > 0) {
    let cur = classes[0] ?? 0;
    let count = 0;
    for (let i = 0; i < total; i++) {
      const c = classes[i] ?? 0;
      if (c === cur && count < 0xffff) {
        count++;
      } else {
        runs.push([count, cur]);
        cur = c;
        count = 1;
      }
    }
    if (count > 0) {
      runs.push([count, cur]);
    }
  }
  const buf = new Uint8Array(4 + runs.length * 3);
  const view = new DataView(buf.buffer);
  view.setUint16(0, width, true);
  view.setUint16(2, height, true);
  let offset = 4;
  for (const [count, code] of runs) {
    view.setUint16(offset, count, true);
    buf[offset + 2] = code & 0xff;
    offset += 3;
  }
  return bytesToBase64(buf);
}

/**
 * Decode `HeightfieldDef.classes` 3-byte run format (`[cols u16 LE][rows u16 LE] + [count u16 LE][class u8]`).
 */
export function decodeClassesBase64(b64: string): {
  width: number;
  height: number;
  classes: Uint8Array;
} {
  const bytes = base64ToBytes(b64);
  if (bytes.length < 4 || (bytes.length - 4) % 3 !== 0) {
    throw new Error('Invalid classes buffer length');
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const width = view.getUint16(0, true);
  const height = view.getUint16(2, true);
  const total = width * height;
  const classes = new Uint8Array(total);
  let pos = 0;
  for (let offset = 4; offset < bytes.length; offset += 3) {
    const count = view.getUint16(offset, true);
    const code = bytes[offset + 2] ?? 0;
    if (count === 0 || pos + count > total) {
      throw new Error('Class run exceeds grid dimensions');
    }
    classes.fill(code, pos, pos + count);
    pos += count;
  }
  if (pos !== total) {
    throw new Error(`Decoded ${pos} class cells, expected ${total}`);
  }
  return { width, height, classes };
}

/**
 * Encode `elevCm` as `le16` (`cols * rows` little-endian `i16` samples, base64-encoded).
 */
export function encodeElevLe16Base64(elevCm: Int16Array): string {
  const buf = new Uint8Array(elevCm.length * 2);
  const view = new DataView(buf.buffer);
  for (let i = 0; i < elevCm.length; i++) {
    view.setInt16(i * 2, elevCm[i] ?? 0, true);
  }
  return bytesToBase64(buf);
}

/**
 * Decode `le16` base64 string into an `Int16Array`.
 */
export function decodeElevLe16Base64(b64: string, expectedCells?: number): Int16Array {
  const bytes = base64ToBytes(b64);
  if (bytes.length % 2 !== 0) {
    throw new Error('le16 elevation byte length must be even');
  }
  const count = bytes.length / 2;
  if (expectedCells !== undefined && count !== expectedCells) {
    throw new Error(`Decoded ${count} elevation samples, expected ${expectedCells}`);
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const out = new Int16Array(count);
  for (let i = 0; i < count; i++) {
    out[i] = view.getInt16(i * 2, true);
  }
  return out;
}

/**
 * Encode `elevCm` in `arena.map.elev`'s `zigzag-leb128-delta` format (`D-30-6`),
 * matching `tacsim::match_::cockpit::elev_field` byte-for-byte.
 */
export function encodeElevZigzagDelta(elevCm: Int16Array): {
  encoding: 'zigzag-leb128-delta';
  unit_cm: 1;
  data: string;
} {
  const bytes: number[] = [];
  let last = 0;
  for (let i = 0; i < elevCm.length; i++) {
    const elev = elevCm[i] ?? 0;
    const delta = (elev - last) | 0;
    last = elev;
    let zig = ((delta << 1) ^ (delta >> 31)) >>> 0;
    while (true) {
      const low = zig & 0x7f;
      zig >>>= 7;
      if (zig === 0) {
        bytes.push(low);
        break;
      }
      bytes.push(low | 0x80);
    }
  }
  return {
    encoding: 'zigzag-leb128-delta',
    unit_cm: 1,
    data: bytesToBase64(new Uint8Array(bytes))
  };
}

/**
 * Export the current studio state as a `ground.draft.v1` JSON document (`D-30-2`, `D-30-8`).
 */
export function exportGroundDraftJson(state: EditorMapState): GroundDraftJson {
  const out: GroundDraftJson = {
    schema: GROUND_DRAFT_SCHEMA,
    id: state.id,
    label: state.label,
    biome: state.biome,
    body: state.body,
    origin_m: [state.originM[0], state.originM[1]],
    cell_cm: state.cellCm,
    cols: state.width,
    rows: state.height,
    classes: encodeClassesBase64(state.width, state.height, state.classes),
    elev: encodeElevLe16Base64(state.elevCm),
    entry_zones: state.entryZones.map((z) => ({
      team: z.team,
      x0: Math.min(z.x0, z.x1),
      y0: Math.min(z.y0, z.y1),
      x1: Math.max(z.x0, z.x1),
      y1: Math.max(z.y0, z.y1)
    })),
    key_spots: state.keySpots.map((s) => ({
      name: s.name,
      x: s.x,
      y: s.y
    })),
    waypoints: {
      nodes: state.waypoints.nodes.map((n) => ({ id: n.id, x: n.x, y: n.y })),
      edges: state.waypoints.edges.map((e) => [e[0], e[1]])
    },
    ...(state.modes.length > 0 ? { modes: state.modes.map((m) => ({ ...m })) } : {}),
    ...(state.tanks.length > 0 ? { tanks: state.tanks.map((t) => ({ ...t })) } : {}),
    atlas_version: state.atlasVersion,
    arena_version: state.arenaVersion,
    provenance: {
      source: state.provenance.source,
      version: state.provenance.version,
      ...(state.provenance.seed !== undefined ? { seed: state.provenance.seed } : {}),
      ...(state.provenance.inputs && state.provenance.inputs.length > 0
        ? { inputs: [...state.provenance.inputs] }
        : {})
    },
    ...(state.note ? { note: state.note } : {})
  };
  return out;
}

/**
 * Import a `ground.draft.v1` JSON document into an `EditorMapState`.
 */
export function importGroundDraftJson(draft: GroundDraftJson): EditorMapState {
  if (draft.schema !== GROUND_DRAFT_SCHEMA) {
    throw new Error(`Unsupported draft schema: ${draft.schema}`);
  }
  const { width, height, classes } = decodeClassesBase64(draft.classes);
  if (width !== draft.cols || height !== draft.rows) {
    throw new Error(
      `Draft cols/rows (${draft.cols}x${draft.rows}) do not match classes header (${width}x${height})`
    );
  }
  const elevCm = decodeElevLe16Base64(draft.elev, width * height);
  return {
    id: draft.id || 'custom_arena',
    label: draft.label || 'Custom Arena',
    biome: BIOME_OPTIONS.includes(draft.biome) ? draft.biome : 'temperate',
    body: draft.body || 'luna',
    width,
    height,
    cellCm: draft.cell_cm || 1000,
    originM: Array.isArray(draft.origin_m)
      ? [draft.origin_m[0] ?? 0, draft.origin_m[1] ?? 0]
      : [0, 0],
    classes,
    elevCm,
    keySpots: (draft.key_spots ?? []).map((s) => ({
      name: s.name,
      x: s.x,
      y: s.y
    })),
    entryZones: (draft.entry_zones ?? []).map((z) => ({
      team: z.team,
      x0: z.x0,
      y0: z.y0,
      x1: z.x1,
      y1: z.y1
    })),
    waypoints: {
      nodes: (draft.waypoints?.nodes ?? []).map((n) => ({ id: n.id, x: n.x, y: n.y })),
      edges: (draft.waypoints?.edges ?? []).map((e) => [e[0], e[1]])
    },
    modes: (draft.modes ?? []).map((m) => ({ ...m })),
    tanks: (draft.tanks ?? []).map((t) => ({ ...t })),
    atlasVersion: draft.atlas_version ?? 1,
    arenaVersion: draft.arena_version ?? 1,
    provenance: {
      source: draft.provenance?.source ?? 'studio',
      version: draft.provenance?.version ?? 'v1',
      ...(draft.provenance?.seed !== undefined ? { seed: draft.provenance.seed } : {}),
      ...(draft.provenance?.inputs ? { inputs: [...draft.provenance.inputs] } : {})
    },
    note: draft.note ?? ''
  };
}

/**
 * Build live `arena.map` and `arena.cockpit` feeds from the current `EditorMapState`
 * and camera pin so `mush-pixel-wasm` renders the exact 1st-person, 2.5D, and overhead
 * views matching the server (`D-30-6`, `D-30-8`).
 */
export function editorToPictureFeeds(
  state: EditorMapState,
  pin: EditorCameraPin
): { arenaMap: Record<string, unknown>; arenaCockpit: Record<string, unknown> } {
  const rows: [string, number][][] = [];
  const seenCodes: number[] = [];

  for (let y = 0; y < state.height; y++) {
    const rowRuns: [string, number][] = [];
    let currentGlyph = '';
    let runLen = 0;
    for (let x = 0; x < state.width; x++) {
      const code = state.classes[y * state.width + x] ?? 2;
      if (!seenCodes.includes(code)) {
        seenCodes.push(code);
      }
      const meta = CLASS_META[code] ?? { glyph: '.', word: 'ground' };
      if (meta.glyph === currentGlyph) {
        runLen++;
      } else {
        if (runLen > 0) {
          rowRuns.push([currentGlyph, runLen]);
        }
        currentGlyph = meta.glyph;
        runLen = 1;
      }
    }
    if (runLen > 0) {
      rowRuns.push([currentGlyph, runLen]);
    }
    rows.push(rowRuns);
  }

  const legend: [string, string][] = seenCodes.map((code) => {
    const meta = CLASS_META[code] ?? { glyph: '.', word: 'ground' };
    return [meta.glyph, meta.word];
  });

  const spots = state.keySpots.slice(0, 26).map((s, i) => ({
    letter: String.fromCharCode(65 + i),
    name: s.name,
    cell: [s.x, s.y]
  }));

  const pinX = Math.max(0, Math.min(state.width - 1, Math.floor(pin.x)));
  const pinY = Math.max(0, Math.min(state.height - 1, Math.floor(pin.y)));
  const pinElev = state.elevCm[pinY * state.width + pinX] ?? 0;
  const headingMdeg = Math.round((((pin.headingDeg % 360) + 360) % 360) * 1000) % 360000;
  const xCm = Math.round((pinX + 0.5) * state.cellCm);
  const yCm = Math.round((pinY + 0.5) * state.cellCm);

  return {
    arenaMap: {
      schema: {
        id: 'arena.map',
        version: 1,
        shape: 'terrain',
        retain: 'latest'
      },
      match: -1,
      surface: `tacsim.arena.${state.id}`,
      biome: state.biome,
      cell_cm: state.cellCm,
      width: state.width,
      height: state.height,
      origin_cell: [0, 0],
      rows,
      legend,
      spots,
      elev: encodeElevZigzagDelta(state.elevCm)
    },
    arenaCockpit: {
      schema: {
        id: 'arena.cockpit',
        version: 8,
        retain: 'latest'
      },
      seated: true,
      match: -1,
      unit: 1,
      callsign: 'PREVIEW',
      cell: [pinX, pinY],
      x_cm: xCm,
      y_cm: yCm,
      z_cm: pinElev,
      heading_mdeg: headingMdeg,
      heading_deg: Math.floor(headingMdeg / 1000),
      speed_cms: 0,
      speed_kph: 0,
      power: 'online',
      sight_m: 600,
      contacts: [],
      weapons: [],
      sections: []
    }
  };
}

// ---------------------------------------------------------------------------
// Physics, Slope, Passability, Check Report, Route & Viewshed (WG.4)
// ---------------------------------------------------------------------------

interface LocomotionRowSpec {
  default: number | null;
  max_slope_pct: number;
  climb_cm?: number;
  classes: Record<string, number | null>;
}

const LOCOMOTIONS = (
  locomotionData as { locomotions: Record<DriveName, LocomotionRowSpec> }
).locomotions;

/**
 * Flat terrain cost for `(drive, classCode)` from `locomotion.json`, or `0` when impassable.
 */
export function moveCostForDrive(drive: DriveName, classCode: number): number {
  const row = LOCOMOTIONS[drive];
  if (!row) return 0;
  const word = CLASS_META[classCode]?.word ?? 'void';
  if (Object.prototype.hasOwnProperty.call(row.classes, word)) {
    const val = row.classes[word];
    return val === null || val === undefined ? 0 : val;
  }
  return row.default === null || row.default === undefined ? 0 : row.default;
}

export function maxSlopePctForDrive(drive: DriveName): number {
  return LOCOMOTIONS[drive]?.max_slope_pct ?? 45;
}

/**
 * Exact `mush_atlas::physics::slope_pct` (`crates/mush-atlas/src/physics.rs:413`):
 * `rise = toElev - fromElev`; if `rise <= 0` return `0`;
 * `run = max(1, isDiagonal ? trunc(cellCm * 141 / 100) : cellCm)`;
 * `return trunc(rise * 100 / run)`.
 */
export function slopePct(
  fromElevCm: number,
  toElevCm: number,
  isDiagonal: boolean,
  cellCm: number
): number {
  const rise = toElevCm - fromElevCm;
  if (rise <= 0) return 0;
  const run = Math.max(1, isDiagonal ? Math.trunc((cellCm * 141) / 100) : cellCm);
  return Math.trunc((rise * 100) / run);
}

/**
 * Exact `tacsim::arena::slope::steepness_pct` (`crates/tacsim/src/arena/slope.rs:146`):
 * symmetric max of `slopePct(a, b)` and `slopePct(b, a)`.
 */
export function steepnessPct(
  elevA: number,
  elevB: number,
  isDiagonal: boolean,
  cellCm: number
): number {
  return Math.max(
    slopePct(elevA, elevB, isDiagonal, cellCm),
    slopePct(elevB, elevA, isDiagonal, cellCm)
  );
}

/**
 * Compute per-cell passability overlay for `drive`:
 * - `0` = impassable terrain class (`moveCost === 0`)
 * - `1` = passable (`max orthogonal grade <= maxSlopePct`)
 * - `2` = steep (`moveCost > 0`, but has an orthogonal passable neighbour with grade > `maxSlopePct`)
 */
export function computePassabilityOverlay(
  state: EditorMapState,
  drive: DriveName
): Uint8Array {
  const w = state.width;
  const h = state.height;
  const limit = maxSlopePctForDrive(drive);
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const idx = y * w + x;
      const cost = moveCostForDrive(drive, state.classes[idx] ?? 0);
      if (cost === 0) {
        out[idx] = 0;
        continue;
      }
      const hereElev = state.elevCm[idx] ?? 0;
      let steep = false;
      for (const [dx, dy] of [
        [1, 0],
        [-1, 0],
        [0, 1],
        [0, -1]
      ] as const) {
        const nx = x + dx;
        const ny = y + dy;
        if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
        const nIdx = ny * w + nx;
        if (moveCostForDrive(drive, state.classes[nIdx] ?? 0) === 0) continue;
        if (steepnessPct(hereElev, state.elevCm[nIdx] ?? 0, false, state.cellCm) > limit) {
          steep = true;
          break;
        }
      }
      out[idx] = steep ? 2 : 1;
    }
  }
  return out;
}

function zoneSeedCells(
  state: EditorMapState,
  zone: EditorEntryZone,
  drive: DriveName
): [number, number][] {
  const seeds: [number, number][] = [];
  const x0 = Math.min(zone.x0, zone.x1);
  const x1 = Math.max(zone.x0, zone.x1);
  const y0 = Math.min(zone.y0, zone.y1);
  const y1 = Math.max(zone.y0, zone.y1);
  const passable = (x: number, y: number) =>
    x >= 0 &&
    x < state.width &&
    y >= 0 &&
    y < state.height &&
    moveCostForDrive(drive, state.classes[y * state.width + x] ?? 0) > 0;

  for (const t of state.tanks) {
    if (
      t.team === zone.team &&
      t.x >= x0 &&
      t.x <= x1 &&
      t.y >= y0 &&
      t.y <= y1 &&
      passable(t.x, t.y)
    ) {
      seeds.push([t.x, t.y]);
    }
  }
  const cx = Math.trunc((x0 + x1) / 2);
  const cy = Math.trunc((y0 + y1) / 2);
  if (passable(cx, cy) && !seeds.some(([sx, sy]) => sx === cx && sy === cy)) {
    seeds.push([cx, cy]);
  }
  if (seeds.length === 0) {
    let bestDist = Infinity;
    let bestCell: [number, number] | null = null;
    for (let y = y0; y <= y1; y++) {
      for (let x = x0; x <= x1; x++) {
        if (passable(x, y)) {
          const dist = Math.abs(x - cx) + Math.abs(y - cy);
          if (dist < bestDist) {
            bestDist = dist;
            bestCell = [x, y];
          }
        }
      }
    }
    if (bestCell) {
      seeds.push(bestCell);
    }
  }
  return seeds;
}

function floodFromSeeds(
  state: EditorMapState,
  drive: DriveName,
  limit: number,
  seeds: [number, number][]
): Uint8Array {
  const w = state.width;
  const h = state.height;
  const seen = new Uint8Array(w * h);
  const queue: [number, number][] = [];
  for (const [sx, sy] of seeds) {
    const idx = sy * w + sx;
    if (seen[idx] === 0) {
      seen[idx] = 1;
      queue.push([sx, sy]);
    }
  }
  let head = 0;
  while (head < queue.length) {
    const [x, y] = queue[head++]!;
    const hereElev = state.elevCm[y * w + x] ?? 0;
    for (const [dx, dy] of [
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1]
    ] as const) {
      const nx = x + dx;
      const ny = y + dy;
      if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
      const nIdx = ny * w + nx;
      if (seen[nIdx] !== 0) continue;
      if (moveCostForDrive(drive, state.classes[nIdx] ?? 0) === 0) continue;
      if (steepnessPct(hereElev, state.elevCm[nIdx] ?? 0, false, state.cellCm) > limit) continue;
      seen[nIdx] = 1;
      queue.push([nx, ny]);
    }
  }
  return seen;
}

const NEIGHBOURS_8: readonly [number, number][] = [
  [0, -1],
  [-1, 0],
  [1, 0],
  [0, 1],
  [-1, -1],
  [1, -1],
  [-1, 1],
  [1, 1]
] as const;

const DIAGONAL_NUM = 181;
const DIAGONAL_DEN = 128;

function minCostOnGrid(state: EditorMapState, drive: DriveName): number {
  let min = 100;
  for (let i = 0; i < state.classes.length; i++) {
    const c = moveCostForDrive(drive, state.classes[i] ?? 0);
    if (c > 0 && c < min) {
      min = c;
    }
  }
  return Math.max(1, min);
}

/**
 * Exact `tacsim::arena::path::path_with` A* search (`crates/tacsim/src/arena/path.rs`).
 */
export function findArenaPath(
  state: EditorMapState,
  start: [number, number],
  goal: [number, number],
  drive: DriveName,
  maxSlopePct: number | null = maxSlopePctForDrive(drive)
): [number, number][] | null {
  const w = state.width;
  const h = state.height;
  const [sx, sy] = start;
  const [gx, gy] = goal;
  if (sx < 0 || sx >= w || sy < 0 || sy >= h || gx < 0 || gx >= w || gy < 0 || gy >= h) {
    return null;
  }
  if (moveCostForDrive(drive, state.classes[sy * w + sx] ?? 0) === 0) return null;
  if (moveCostForDrive(drive, state.classes[gy * w + gx] ?? 0) === 0) return null;
  if (sx === gx && sy === gy) return [];

  const scale = minCostOnGrid(state, drive);
  const heuristic = (x: number, y: number): number => {
    const dx = Math.abs(x - gx);
    const dy = Math.abs(y - gy);
    const long = Math.max(dx, dy);
    const short = Math.min(dx, dy);
    const straight = (long - short) * scale;
    const diagonal = Math.trunc((short * scale * DIAGONAL_NUM) / DIAGONAL_DEN);
    return straight + diagonal;
  };

  const total = w * h;
  const bestCost = new Uint32Array(total);
  bestCost.fill(0xffffffff);
  const parent = new Int32Array(total);
  parent.fill(-1);

  const startIdx = sy * w + sx;
  const goalIdx = gy * w + gx;
  bestCost[startIdx] = 0;

  // Binary min-heap ordered by (estimate, nodeIndex) ascending
  const heapEst: number[] = [heuristic(sx, sy)];
  const heapNode: number[] = [startIdx];

  const heapPush = (est: number, node: number) => {
    let i = heapEst.length;
    heapEst.push(est);
    heapNode.push(node);
    while (i > 0) {
      const p = (i - 1) >> 1;
      const pEst = heapEst[p]!;
      const pNode = heapNode[p]!;
      if (pEst < est || (pEst === est && pNode <= node)) {
        break;
      }
      heapEst[i] = pEst;
      heapNode[i] = pNode;
      i = p;
    }
    heapEst[i] = est;
    heapNode[i] = node;
  };

  const heapPop = (): [number, number] | null => {
    if (heapEst.length === 0) return null;
    const topEst = heapEst[0]!;
    const topNode = heapNode[0]!;
    const lastEst = heapEst.pop()!;
    const lastNode = heapNode.pop()!;
    if (heapEst.length > 0) {
      let i = 0;
      const n = heapEst.length;
      while (true) {
        const l = 2 * i + 1;
        const r = l + 1;
        let smallest = i;
        const sEst0 = smallest === i ? lastEst : heapEst[smallest]!;
        const sNode0 = smallest === i ? lastNode : heapNode[smallest]!;
        if (
          l < n &&
          (heapEst[l]! < sEst0 ||
            (heapEst[l]! === sEst0 && heapNode[l]! < sNode0))
        ) {
          smallest = l;
        }
        const curEst = smallest === i ? lastEst : heapEst[smallest]!;
        const curNode = smallest === i ? lastNode : heapNode[smallest]!;
        if (
          r < n &&
          (heapEst[r]! < curEst || (heapEst[r]! === curEst && heapNode[r]! < curNode))
        ) {
          smallest = r;
        }
        if (smallest === i) break;
        heapEst[i] = heapEst[smallest]!;
        heapNode[i] = heapNode[smallest]!;
        i = smallest;
      }
      heapEst[i] = lastEst;
      heapNode[i] = lastNode;
    }
    return [topEst, topNode];
  };

  while (heapEst.length > 0) {
    const popped = heapPop()!;
    const node = popped[1];
    const cost = bestCost[node]!;
    if (node === goalIdx) {
      const cells: [number, number][] = [];
      let cur = goalIdx;
      while (cur !== startIdx) {
        const cx = cur % w;
        const cy = (cur - cx) / w;
        cells.push([cx, cy]);
        cur = parent[cur] ?? -1;
        if (cur < 0) return null;
      }
      cells.reverse();
      return cells;
    }

    const x = node % w;
    const y = (node - x) / w;
    const hereElev = state.elevCm[node] ?? 0;

    for (const [dx, dy] of NEIGHBOURS_8) {
      const nx = x + dx;
      const ny = y + dy;
      if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
      const nextIdx = ny * w + nx;
      const baseStep = moveCostForDrive(drive, state.classes[nextIdx] ?? 0);
      if (baseStep === 0) continue;

      const isDiag = dx !== 0 && dy !== 0;
      const thereElev = state.elevCm[nextIdx] ?? 0;
      const grade = slopePct(hereElev, thereElev, isDiag, state.cellCm);

      if (maxSlopePct !== null) {
        if (steepnessPct(hereElev, thereElev, isDiag, state.cellCm) > maxSlopePct) {
          continue;
        }
        if (isDiag) {
          const s1Idx = y * w + nx;
          const s2Idx = ny * w + x;
          const s1Pass = moveCostForDrive(drive, state.classes[s1Idx] ?? 0) > 0;
          const s2Pass = moveCostForDrive(drive, state.classes[s2Idx] ?? 0) > 0;
          const s1Elev = state.elevCm[s1Idx] ?? 0;
          const s2Elev = state.elevCm[s2Idx] ?? 0;
          const s1Steep =
            s1Pass &&
            (steepnessPct(hereElev, s1Elev, false, state.cellCm) > maxSlopePct ||
              steepnessPct(s1Elev, thereElev, false, state.cellCm) > maxSlopePct);
          const s2Steep =
            s2Pass &&
            (steepnessPct(hereElev, s2Elev, false, state.cellCm) > maxSlopePct ||
              steepnessPct(s2Elev, thereElev, false, state.cellCm) > maxSlopePct);
          if (s1Steep || s2Steep) continue;
          const s1Open = s1Pass && !s1Steep;
          const s2Open = s2Pass && !s2Steep;
          if (!s1Open && !s2Open) continue;
        }
      } else if (isDiag) {
        const s1Pass = moveCostForDrive(drive, state.classes[y * w + nx] ?? 0) > 0;
        const s2Pass = moveCostForDrive(drive, state.classes[ny * w + x] ?? 0) > 0;
        if (!s1Pass && !s2Pass) continue;
      }

      const surcharged = Math.trunc((baseStep * (100 + grade)) / 100);
      const stepCost = isDiag
        ? Math.trunc((surcharged * DIAGONAL_NUM) / DIAGONAL_DEN)
        : surcharged;
      const nextCost = cost + stepCost;
      if (nextCost < (bestCost[nextIdx] ?? 0xffffffff)) {
        bestCost[nextIdx] = nextCost;
        parent[nextIdx] = node;
        heapPush(nextCost + heuristic(nx, ny), nextIdx);
      }
    }
  }
  return null;
}

/**
 * Exact §5 grade and passability check report (`tacsim::ground::check`).
 */
export function checkEditorMap(state: EditorMapState): GroundCheckReport {
  const feeds = editorToPictureFeeds(state, { x: 0, y: 0, headingDeg: 0 });
  const mapFrame = {
    ...feeds.arenaMap,
    match: 0
  };
  // In Rust `WIDEST_MATCH` is `u64::MAX` (`18446744073709551615`, 20 chars vs `0`'s 1 char = +19 bytes).
  const mapBytes = new TextEncoder().encode(JSON.stringify(mapFrame)).length + 19;
  const quadrants = Math.ceil(state.width / 128) * Math.ceil(state.height / 128);

  const trackedLimit = maxSlopePctForDrive('tracked');
  const wheeledLimit = maxSlopePctForDrive('wheeled');
  const leggedLimit = maxSlopePctForDrive('legged');

  const isLand = (code: number) => code !== 0 && code !== 4 && code !== 15 && code !== 10;
  let landTotal = 0;
  for (let i = 0; i < state.classes.length; i++) {
    if (isLand(state.classes[i] ?? 0)) landTotal++;
  }

  let trackedPctMin = 100;
  let wheeledPctMin = 100;
  let leggedPctMin = 100;

  const totalCells = state.width * state.height;
  const trackedUnion = new Uint8Array(totalCells);
  const wheeledUnion = new Uint8Array(totalCells);

  const zoneSeedsTracked: ([number, number] | null)[] = [];
  const zoneSeedsWheeled: ([number, number] | null)[] = [];
  const zoneSeedsLegged: ([number, number] | null)[] = [];

  for (const zone of state.entryZones) {
    const tSeeds = zoneSeedCells(state, zone, 'tracked');
    const wSeeds = zoneSeedCells(state, zone, 'wheeled');
    const lSeeds = zoneSeedCells(state, zone, 'legged');

    zoneSeedsTracked.push(tSeeds[0] ?? null);
    zoneSeedsWheeled.push(wSeeds[0] ?? null);
    zoneSeedsLegged.push(lSeeds[0] ?? null);

    const tFlood = floodFromSeeds(state, 'tracked', trackedLimit, tSeeds);
    const wFlood = floodFromSeeds(state, 'wheeled', wheeledLimit, wSeeds);
    const lFlood = floodFromSeeds(state, 'legged', leggedLimit, lSeeds);

    const countLandPct = (flood: Uint8Array): number => {
      if (landTotal === 0) return 100;
      let reached = 0;
      for (let i = 0; i < totalCells; i++) {
        if (flood[i] && isLand(state.classes[i] ?? 0)) reached++;
      }
      return Math.trunc((reached * 100) / landTotal);
    };

    trackedPctMin = Math.min(trackedPctMin, countLandPct(tFlood));
    wheeledPctMin = Math.min(wheeledPctMin, countLandPct(wFlood));
    leggedPctMin = Math.min(leggedPctMin, countLandPct(lFlood));

    for (let i = 0; i < totalCells; i++) {
      if (tFlood[i]) trackedUnion[i] = 1;
      if (wFlood[i]) wheeledUnion[i] = 1;
    }
  }

  const mutual = (seeds: ([number, number] | null)[], drive: DriveName, limit: number): boolean => {
    const a = seeds[0];
    const b = seeds[1];
    if (!a || !b) return false;
    return findArenaPath(state, a, b, drive, limit) !== null;
  };

  const zonesReachableTracked = mutual(zoneSeedsTracked, 'tracked', trackedLimit);
  const zonesReachableWheeled = mutual(zoneSeedsWheeled, 'wheeled', wheeledLimit);
  const zonesReachableLegged = mutual(zoneSeedsLegged, 'legged', leggedLimit);

  const unreachableKeySpots: string[] = [];
  for (const spot of state.keySpots) {
    if (
      spot.x < 0 ||
      spot.x >= state.width ||
      spot.y < 0 ||
      spot.y >= state.height ||
      wheeledUnion[spot.y * state.width + spot.x] === 0
    ) {
      unreachableKeySpots.push(spot.name);
    }
  }
  const keySpotsWheeledReachable = unreachableKeySpots.length === 0;

  const unreachableWaypoints: string[] = [];
  for (const node of state.waypoints.nodes) {
    if (
      node.x < 0 ||
      node.x >= state.width ||
      node.y < 0 ||
      node.y >= state.height ||
      trackedUnion[node.y * state.width + node.x] === 0
    ) {
      unreachableWaypoints.push(node.id);
    }
  }
  const waypointsReachable = unreachableWaypoints.length === 0;

  let maxGradePct = 0;
  let maxRoadGradePct = 0;
  let hasRoad = false;
  const w = state.width;
  const h = state.height;

  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const idx = y * w + x;
      const cHere = state.classes[idx] ?? 0;
      const hereRoad = cHere === 7 || cHere === 1;
      if (hereRoad) hasRoad = true;
      const herePass = moveCostForDrive('tracked', cHere) > 0;
      const hereElev = state.elevCm[idx] ?? 0;

      for (const [nx, ny] of [
        [x + 1, y],
        [x, y + 1]
      ] as const) {
        if (nx >= w || ny >= h) continue;
        const nIdx = ny * w + nx;
        const cThere = state.classes[nIdx] ?? 0;
        const therePass = moveCostForDrive('tracked', cThere) > 0;
        const steep = steepnessPct(hereElev, state.elevCm[nIdx] ?? 0, false, state.cellCm);
        if (herePass && therePass && steep > maxGradePct) {
          maxGradePct = steep;
        }
        if (hereRoad && (cThere === 7 || cThere === 1) && steep > maxRoadGradePct) {
          maxRoadGradePct = steep;
        }
      }
    }
  }

  let maxEntryZoneStepCm = 0;
  for (const zone of state.entryZones) {
    const x0 = Math.min(zone.x0, zone.x1);
    const x1 = Math.max(zone.x0, zone.x1);
    const y0 = Math.min(zone.y0, zone.y1);
    const y1 = Math.max(zone.y0, zone.y1);
    const seeds = zoneSeedCells(state, zone, 'tracked');
    for (const [sx, sy] of seeds) {
      const hereElev = state.elevCm[sy * w + sx] ?? 0;
      for (const [dx, dy] of [
        [1, 0],
        [-1, 0],
        [0, 1],
        [0, -1]
      ] as const) {
        const nx = sx + dx;
        const ny = sy + dy;
        if (nx < x0 || nx > x1 || ny < y0 || ny > y1) continue;
        if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
        const step = Math.abs(hereElev - (state.elevCm[ny * w + nx] ?? 0));
        if (step > maxEntryZoneStepCm) {
          maxEntryZoneStepCm = step;
        }
      }
    }
  }

  const failures: string[] = [];
  if (mapBytes > MAP_MAX_BYTES) {
    failures.push(`arena.map is ${mapBytes} bytes (max ${MAP_MAX_BYTES})`);
  }
  if (trackedPctMin < 90) {
    failures.push(`tracked reachability is ${trackedPctMin}% (need >= 90%)`);
  }
  if (leggedPctMin < 95) {
    failures.push(`legged reachability is ${leggedPctMin}% (need >= 95%)`);
  }
  if (!zonesReachableTracked) {
    failures.push('entry zones are not mutually reachable by tracked');
  }
  if (!zonesReachableLegged) {
    failures.push('entry zones are not mutually reachable by legged');
  }
  if (!zonesReachableWheeled && (state.keySpots.length > 0 || hasRoad)) {
    failures.push('entry zones are not mutually reachable by wheeled');
  }
  if (!keySpotsWheeledReachable) {
    failures.push(`key spots not reachable by wheeled: ${unreachableKeySpots.join(', ')}`);
  }
  if (!waypointsReachable) {
    failures.push(`waypoints not reachable by tracked: ${unreachableWaypoints.join(', ')}`);
  }
  if (maxRoadGradePct > 20) {
    failures.push(`max road grade is ${maxRoadGradePct}% (need <= 20%)`);
  }
  const stepLimitCm = state.provenance?.version === 'v3' ? 100 : 50;
  if (maxEntryZoneStepCm > stepLimitCm) {
    failures.push(`entry zone muster step is ${maxEntryZoneStepCm} cm (need <= ${stepLimitCm} cm)`);
  }

  return {
    id: state.id,
    cols: state.width,
    rows: state.height,
    quadrants,
    map_bytes: mapBytes,
    map_max_bytes: MAP_MAX_BYTES,
    tracked_passable_pct: trackedPctMin,
    wheeled_passable_pct: wheeledPctMin,
    legged_passable_pct: leggedPctMin,
    zones_reachable_tracked: zonesReachableTracked,
    zones_reachable_wheeled: zonesReachableWheeled,
    zones_reachable_legged: zonesReachableLegged,
    key_spots_wheeled_reachable: keySpotsWheeledReachable,
    unreachable_key_spots: unreachableKeySpots,
    waypoints_reachable: waypointsReachable,
    unreachable_waypoints: unreachableWaypoints,
    max_grade_pct: maxGradePct,
    max_road_grade_pct: maxRoadGradePct,
    max_entry_zone_step_cm: maxEntryZoneStepCm,
    passed: failures.length === 0,
    failures
  };
}

/**
 * Auto-route waypoint synthesis (`tacsim::ground::route`, doc 30 §5 & §8):
 * places nodes at each entry zone's seed cell and each key spot, connects them
 * by A* over the tracked cost grid with a node every 20 cells, and writes
 * `state.waypoints`.
 */
export function routeEditorMap(state: EditorMapState): EditorWaypointGraph {
  const trackedLimit = maxSlopePctForDrive('tracked');
  const nodes: EditorWaypointNode[] = [];
  const edges: [string, string][] = [];
  const edgeSet = new Set<string>();

  const findOrInsertNode = (preferredId: string, x: number, y: number): string => {
    const existing = nodes.find((n) => Math.abs(n.x - x) <= 2 && Math.abs(n.y - y) <= 2);
    if (existing) return existing.id;
    if (nodes.length >= 256) {
      let best = nodes[0]!;
      let bestDist = Infinity;
      for (const n of nodes) {
        const d = Math.abs(n.x - x) + Math.abs(n.y - y);
        if (d < bestDist) {
          bestDist = d;
          best = n;
        }
      }
      return best.id;
    }
    nodes.push({ id: preferredId, x, y });
    return preferredId;
  };

  const zoneAnchors: [number, number][] = [];
  for (const z of state.entryZones) {
    const seeds = zoneSeedCells(state, z, 'tracked');
    const [zx, zy] = seeds[0] ?? [
      Math.trunc((z.x0 + z.x1) / 2),
      Math.trunc((z.y0 + z.y1) / 2)
    ];
    findOrInsertNode(`muster_${z.team}`, zx, zy);
    zoneAnchors.push([zx, zy]);
  }

  const spotAnchors: [number, number][] = [];
  state.keySpots.forEach((spot, idx) => {
    const id = `spot_${idx}`;
    if (!nodes.some((n) => n.x === spot.x && n.y === spot.y) && nodes.length < 256) {
      nodes.push({ id, x: spot.x, y: spot.y });
    }
    spotAnchors.push([spot.x, spot.y]);
  });

  const pairs: [[number, number], [number, number]][] = [];
  if (spotAnchors.length === 0) {
    if (zoneAnchors.length >= 2) {
      pairs.push([zoneAnchors[0]!, zoneAnchors[1]!]);
    }
  } else {
    for (const zxy of zoneAnchors) {
      for (const sxy of spotAnchors) {
        pairs.push([zxy, sxy]);
      }
    }
    for (let i = 0; i + 1 < spotAnchors.length; i++) {
      pairs.push([spotAnchors[i]!, spotAnchors[i + 1]!]);
    }
  }

  let stepCounter = 0;
  for (const [start, goal] of pairs) {
    const pathCells = findArenaPath(state, start, goal, 'tracked', trackedLimit);
    if (!pathCells) continue;
    const waypointsAlong: [number, number][] = [start];
    for (let i = 0; i < pathCells.length; i++) {
      const cell = pathCells[i]!;
      if ((i + 1) % 20 === 0 && (cell[0] !== goal[0] || cell[1] !== goal[1])) {
        waypointsAlong.push(cell);
      }
    }
    waypointsAlong.push(goal);

    let prevId: string | null = null;
    for (const [wx, wy] of waypointsAlong) {
      const name = `wp_${String(stepCounter).padStart(3, '0')}`;
      stepCounter++;
      const curId = findOrInsertNode(name, wx, wy);
      if (prevId !== null && prevId !== curId && edges.length < 1024) {
        const key = prevId <= curId ? `${prevId}|${curId}` : `${curId}|${prevId}`;
        if (!edgeSet.has(key)) {
          edgeSet.add(key);
          edges.push([prevId, curId]);
        }
      }
      prevId = curId;
    }
  }

  state.waypoints = { nodes, edges };
  return state.waypoints;
}

/**
 * Exact `mush_atlas::trace::sight_only` ray walk (`crates/mush-atlas/src/trace.rs`),
 * used by `computeViewshed` to match `tacsim::ground::viewshed` / `horizon.rs`.
 */
function traceSightState(
  state: EditorMapState,
  ox: number,
  oy: number,
  fromHeightCm: number,
  tx: number,
  ty: number,
  toHeightCm: number
): 'clear' | 'partial' | 'blocked' {
  // canonical_ends: order by (x, y) ascending
  const swap = tx < ox || (tx === ox && ty < oy);
  const loX = swap ? tx : ox;
  const loY = swap ? ty : oy;
  const loH = swap ? toHeightCm : fromHeightCm;
  const hiX = swap ? ox : tx;
  const hiY = swap ? oy : ty;
  const hiH = swap ? fromHeightCm : toHeightCm;

  const w = state.width;
  const zLo = (state.elevCm[loY * w + loX] ?? 0) + loH;
  const zHi = (state.elevCm[hiY * w + hiX] ?? 0) + hiH;
  const n = Math.max(Math.abs(hiX - loX), Math.abs(hiY - loY));
  if (n <= 1) {
    return 'clear';
  }

  // Exact mush_atlas::trace::Bresenham::new((loX, loY), (hiX, hiY))
  const dx = Math.abs(hiX - loX);
  const dy = Math.abs(hiY - loY);
  const sx = loX < hiX ? 1 : -1;
  const sy = loY < hiY ? 1 : -1;
  const xMajor = dx >= dy;
  let err = xMajor ? Math.trunc(dx / 2) : Math.trunc(dy / 2);
  let x = loX;
  let y = loY;
  let attenuation = 0;

  for (let i = 0; i < n; i++) {
    // Advance one step to interior cell `i + 1`
    if (xMajor) {
      x += sx;
      err -= dy;
      if (err < 0) {
        y += sy;
        err += dx;
      }
    } else {
      y += sy;
      err -= dx;
      if (err < 0) {
        x += sx;
        err += dy;
      }
    }
    const step = i + 1;
    if (step >= n) break;

    const idx = y * w + x;
    const classCode = state.classes[idx] ?? 0;
    // Void (0) or Wall (10) -> Opaque
    if (classCode === 0 || classCode === 10) {
      return 'blocked';
    }
    if (classCode === 13) {
      attenuation += 25;
      if (attenuation >= 100) return 'blocked';
    } else if (classCode === 14) {
      attenuation += 60;
      if (attenuation >= 100) return 'blocked';
    }
    // line_height_cm: trunc((zLo * (n - step) + zHi * step) / n)
    const lineCm = Math.trunc((zLo * (n - step) + zHi * step) / n);
    if ((state.elevCm[idx] ?? 0) > lineCm) {
      return 'blocked';
    }
  }

  return attenuation > 0 ? 'partial' : 'clear';
}

/**
 * Compute viewshed from `(ox, oy)` at `EYE_HEIGHT_CM` (`150 cm`) against target
 * height `sizeCm` (default `300 cm`), returning `Uint8Array(width * height)`:
 * - `0` = dead ground (turret at `sizeCm` blocked)
 * - `1` = seen (clear line of sight to both hull and turret)
 * - `2` = hull-down (turret at `sizeCm` seen/partial, hull at `150 cm` blocked)
 * - `3` = partial (woods cover the line partially, 25% / 60%)
 */
export function computeViewshed(
  state: EditorMapState,
  ox: number,
  oy: number,
  sizeCm = DEFAULT_HULL_SIZE_CM
): Uint8Array {
  const w = state.width;
  const h = state.height;
  const out = new Uint8Array(w * h);
  if (ox < 0 || ox >= w || oy < 0 || oy >= h) return out;

  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const idx = y * w + x;
      if (x === ox && y === oy) {
        out[idx] = 1;
        continue;
      }
      const turret = traceSightState(state, ox, oy, EYE_HEIGHT_CM, x, y, sizeCm);
      if (turret === 'blocked') {
        out[idx] = 0;
        continue;
      }
      const hull = traceSightState(state, ox, oy, EYE_HEIGHT_CM, x, y, EYE_HEIGHT_CM);
      if (hull === 'blocked') {
        out[idx] = 2;
      } else if (turret === 'partial' || hull === 'partial') {
        out[idx] = 3;
      } else {
        out[idx] = 1;
      }
    }
  }
  return out;
}

/**
 * Interactive **Relax Slopes** pass (`tacsim::ground::relax_slopes`, doc 30 §5 & §8):
 * pulls the higher cell of steep orthogonal edges down toward the drive limit
 * (`20%` on roads/paved, `maxGradePct` default `45%` on general passable ground),
 * preserving entry zones, and returns the flat indices of all changed cells.
 */
export function relaxEditorSlopes(state: EditorMapState, maxGradePct = 45): number[] {
  const w = state.width;
  const h = state.height;
  const orig = new Int16Array(state.elevCm);
  const inZone = new Uint8Array(w * h);
  for (const z of state.entryZones) {
    const x0 = Math.max(0, Math.min(z.x0, z.x1));
    const x1 = Math.min(w - 1, Math.max(z.x0, z.x1));
    const y0 = Math.max(0, Math.min(z.y0, z.y1));
    const y1 = Math.min(h - 1, Math.max(z.y0, z.y1));
    for (let y = y0; y <= y1; y++) {
      for (let x = x0; x <= x1; x++) {
        inZone[y * w + x] = 1;
      }
    }
  }

  const isPassable = (c: number) => c !== 0 && c !== 10 && c !== 15 && c !== 14;
  const isRoad = (c: number) => c === 7 || c === 1;
  const roadMaxRise = Math.trunc((state.cellCm * 20) / 100);
  const groundMaxRise = Math.trunc((state.cellCm * maxGradePct) / 100);

  for (let iter = 0; iter < 64; iter++) {
    let changedIter = false;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const a = y * w + x;
        const ca = state.classes[a] ?? 0;
        if (!isPassable(ca)) continue;
        for (const [nx, ny] of [
          [x + 1, y],
          [x, y + 1]
        ] as const) {
          if (nx >= w || ny >= h) continue;
          const b = ny * w + nx;
          const cb = state.classes[b] ?? 0;
          if (!isPassable(cb)) continue;
          const maxRise = isRoad(ca) && isRoad(cb) ? roadMaxRise : groundMaxRise;
          const ea = state.elevCm[a] ?? 0;
          const eb = state.elevCm[b] ?? 0;
          if (Math.abs(ea - eb) <= maxRise) continue;
          const za = inZone[a] !== 0;
          const zb = inZone[b] !== 0;
          if (za && zb) continue;
          if (ea > eb) {
            if (!za) {
              const target = clampElevationCm(eb + maxRise);
              if (state.elevCm[a] !== target) {
                state.elevCm[a] = target;
                changedIter = true;
              }
            } else if (!zb) {
              const target = clampElevationCm(ea - maxRise);
              if (state.elevCm[b] !== target) {
                state.elevCm[b] = target;
                changedIter = true;
              }
            }
          } else if (!zb) {
            const target = clampElevationCm(ea + maxRise);
            if (state.elevCm[b] !== target) {
              state.elevCm[b] = target;
              changedIter = true;
            }
          } else if (!za) {
            const target = clampElevationCm(eb - maxRise);
            if (state.elevCm[a] !== target) {
              state.elevCm[a] = target;
              changedIter = true;
            }
          }
        }
      }
    }
    if (!changedIter) break;
  }

  const changed: number[] = [];
  for (let i = 0; i < orig.length; i++) {
    if (state.elevCm[i] !== orig[i]) {
      changed.push(i);
    }
  }
  return changed;
}

/**
 * Interactive **Despeckle** pass (`tacsim::ground::despeckle`, doc 30 §5 & §8):
 * replaces 4-connected class islands smaller than `minCells` (default `3`) with
 * their majority neighbour class, exempting built classes (`paved`, `road`,
 * `floor`, `door`, `platform`, `wall`) and key-spot cells, and returns the flat
 * indices of all changed cells.
 */
export function despeckleEditorMap(state: EditorMapState, minCells = 3): number[] {
  const w = state.width;
  const h = state.height;
  const orig = new Uint8Array(state.classes);
  const isExemptClass = (c: number) =>
    c === 0 || c === 1 || c === 5 || c === 7 || c === 10 || c === 11 || c === 12;

  const spotCells = new Set<number>();
  for (const s of state.keySpots) {
    if (s.x >= 0 && s.x < w && s.y >= 0 && s.y < h) {
      spotCells.add(s.y * w + s.x);
    }
  }

  const visited = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const start = y * w + x;
      if (visited[start]) continue;
      const cls = state.classes[start] ?? 0;
      if (isExemptClass(cls)) {
        visited[start] = 1;
        continue;
      }
      const component: number[] = [];
      const queue: number[] = [start];
      visited[start] = 1;
      let hasSpot = false;
      let head = 0;
      while (head < queue.length) {
        const cur = queue[head++]!;
        component.push(cur);
        if (spotCells.has(cur)) hasSpot = true;
        const cx = cur % w;
        const cy = (cur - cx) / w;
        for (const [dx, dy] of [
          [1, 0],
          [-1, 0],
          [0, 1],
          [0, -1]
        ] as const) {
          const nx = cx + dx;
          const ny = cy + dy;
          if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
          const nIdx = ny * w + nx;
          if (visited[nIdx]) continue;
          if (state.classes[nIdx] === cls) {
            visited[nIdx] = 1;
            queue.push(nIdx);
          }
        }
      }
      if (hasSpot || component.length >= minCells) continue;

      const neighbourCounts = new Map<number, number>();
      for (const idx of component) {
        const cx = idx % w;
        const cy = (idx - cx) / w;
        for (const [dx, dy] of [
          [1, 0],
          [-1, 0],
          [0, 1],
          [0, -1]
        ] as const) {
          const nx = cx + dx;
          const ny = cy + dy;
          if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
          const nc = state.classes[ny * w + nx] ?? 0;
          if (nc !== cls && nc !== 0) {
            neighbourCounts.set(nc, (neighbourCounts.get(nc) ?? 0) + 1);
          }
        }
      }
      let bestClass: number | null = null;
      let bestCount = -1;
      for (const [nc, count] of neighbourCounts.entries()) {
        if (count > bestCount || (count === bestCount && bestClass !== null && nc < bestClass)) {
          bestCount = count;
          bestClass = nc;
        }
      }
      if (bestClass !== null) {
        for (const idx of component) {
          state.classes[idx] = bestClass;
        }
      }
    }
  }

  const changed: number[] = [];
  for (let i = 0; i < orig.length; i++) {
    if (state.classes[i] !== orig[i]) {
      changed.push(i);
    }
  }
  return changed;
}
