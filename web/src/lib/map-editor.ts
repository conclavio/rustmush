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
  { code: 12, word: 'platform', glyph: 'D', label: 'Platform', color: '#7a828c' }
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
  | 'tanks'
  | 'zone_red'
  | 'zone_blue'
  | 'camera_pin'
  | 'pan';

export type SculptMode = 'raise' | 'lower' | 'flatten' | 'smooth';
export type BrushShape = 'circle' | 'square';
export type DriveName = 'tracked' | 'wheeled' | 'legged';
export type StudioOverlayMode = 'none' | 'passability' | 'viewshed';
export type GeneratorPresetName = 'proving_ground' | 'mars_canyon' | 'lunar_crater' | 'ice_shelf';

export interface GeneratorPresetMeta {
  id: GeneratorPresetName;
  label: string;
  biome: BiomeName;
  body: string;
  defaultSeed: number;
}

export const GENERATOR_PRESETS: readonly GeneratorPresetMeta[] = [
  {
    id: 'proving_ground',
    label: 'The Proving Ground (Temperate V3)',
    biome: 'temperate',
    body: 'luna',
    defaultSeed: 2350
  },
  {
    id: 'mars_canyon',
    label: 'Mars Canyon (Mars Wadi & Shelves)',
    biome: 'mars',
    body: 'mars',
    defaultSeed: 2350
  },
  {
    id: 'lunar_crater',
    label: 'Lunar Crater (Twin Impact Basins)',
    biome: 'lunar',
    body: 'luna',
    defaultSeed: 2350
  },
  {
    id: 'ice_shelf',
    label: 'Ice Shelf (Europa Leads & Dome)',
    biome: 'ice',
    body: 'europa',
    defaultSeed: 2350
  }
] as const;

export const SHIPPED_ARENA_IDS: readonly { id: string; label: string; biome: BiomeName }[] = [
  { id: 'proving_ground', label: 'The Proving Ground (Temperate)', biome: 'temperate' },
  { id: 'winter_valley', label: 'Winter Valley (Unity Temperate)', biome: 'temperate' },
  { id: 'mars_canyon', label: 'Mars Canyon (Mars)', biome: 'mars' }
] as const;

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
  name: string;
  team: 'red' | 'blue';
  x: number;
  y: number;
  heading_mdeg: number;
  unit: string;
}

export interface EditorProvenance {
  source: 'generator' | 'extractor' | 'studio' | 'pack';
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
export const MAX_MUSTER_STEP_CM = 100;
export const MAP_MAX_BYTES = 262_144;
export const EYE_HEIGHT_CM = 150;
export const DEFAULT_HULL_SIZE_CM = 300;

/**
 * Record a studio edit in `state.provenance` (doc 30 §4 & §8):
 * flips `source` to `'studio'`, sets `version` to `'1.0'`, drops `seed`, and
 * preserves prior generator/pack provenance in `inputs`.
 */
export function markStudioEdited(state: EditorMapState): void {
  if (state.provenance.source !== 'studio') {
    const priorTag =
      state.provenance.seed !== undefined
        ? `${state.provenance.source}:${state.provenance.version}:seed=${state.provenance.seed}`
        : `${state.provenance.source}:${state.provenance.version}`;
    const existing = state.provenance.inputs ? [...state.provenance.inputs] : [];
    if (!existing.includes(priorTag)) {
      existing.push(priorTag);
    }
    state.provenance = {
      source: 'studio',
      version: '1.0',
      inputs: existing
    };
  }
}

export function defaultTanksForZones(
  width: number,
  height: number,
  zones: EditorEntryZone[],
  classes?: Uint8Array
): EditorTankDef[] {
  const w = Math.max(16, Math.floor(width));
  const h = Math.max(16, Math.floor(height));
  const midY = Math.max(4, Math.min(h - 5, Math.floor(h / 2)));
  let redZone = zones.find((z) => z.team === 'red');
  let blueZone = zones.find((z) => z.team === 'blue');
  if (!redZone) {
    redZone = { team: 'red', x0: 2, y0: 2, x1: Math.min(w - 1, 15), y1: h - 3 };
  }
  if (!blueZone) {
    blueZone = {
      team: 'blue',
      x0: Math.max(0, w - 16),
      y0: 2,
      x1: w - 3,
      y1: h - 3
    };
  }
  const offsets: readonly [number, number][] = [
    [0, 0],
    [0, -2],
    [0, 2],
    [0, -4],
    [0, 4],
    [-2, -3]
  ];
  const isPassable = (x: number, y: number): boolean => {
    if (x < 0 || x >= w || y < 0 || y >= h) return false;
    if (!classes) return true;
    const code = classes[y * w + x] ?? 2;
    return code !== 0 && code !== 4 && code !== 10 && code !== 14 && code !== 15;
  };
  const placeTeam = (
    team: 'red' | 'blue',
    zone: EditorEntryZone,
    headingMdeg: number,
    flipDx: boolean
  ): EditorTankDef[] => {
    const x0 = Math.max(0, Math.min(zone.x0, zone.x1));
    const x1 = Math.min(w - 1, Math.max(zone.x0, zone.x1));
    const y0 = Math.max(0, Math.min(zone.y0, zone.y1));
    const y1 = Math.min(h - 1, Math.max(zone.y0, zone.y1));
    const cx = Math.floor((x0 + x1) / 2);
    const cy = Math.max(y0, Math.min(y1, midY));
    const used = new Set<string>();
    const out: EditorTankDef[] = [];
    for (let idx = 0; idx < 6; idx++) {
      const slot = idx + 1;
      const [rawDx, rawDy] = offsets[idx]!;
      const dx = flipDx ? -rawDx : rawDx;
      let tx = Math.max(x0, Math.min(x1, cx + dx));
      let ty = Math.max(y0, Math.min(y1, cy + rawDy));
      if (!isPassable(tx, ty) || used.has(`${tx},${ty}`)) {
        let found = false;
        for (let r = 1; r <= Math.max(x1 - x0, y1 - y0) + 2 && !found; r++) {
          for (let oy = -r; oy <= r && !found; oy++) {
            for (let ox = -r; ox <= r && !found; ox++) {
              const nx = tx + ox;
              const ny = ty + oy;
              if (
                nx >= x0 &&
                nx <= x1 &&
                ny >= y0 &&
                ny <= y1 &&
                isPassable(nx, ny) &&
                !used.has(`${nx},${ny}`)
              ) {
                tx = nx;
                ty = ny;
                found = true;
              }
            }
          }
        }
      }
      used.add(`${tx},${ty}`);
      out.push({
        id: `${team}_${slot}`,
        name: `${team === 'red' ? 'Red' : 'Blue'} Tank ${slot}`,
        team,
        x: tx,
        y: ty,
        heading_mdeg: headingMdeg,
        unit: 'tacsim.unit.line_tank'
      });
    }
    return out;
  };
  return [
    ...placeTeam('red', redZone, 90_000, false),
    ...placeTeam('blue', blueZone, 270_000, true)
  ];
}

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
  const entryZones: EditorEntryZone[] = [
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
  ];
  return {
    id: 'custom_arena',
    label: 'Custom Arena',
    biome,
    body: biome === 'mars' ? 'mars' : biome === 'ice' ? 'europa' : 'luna',
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
    entryZones,
    waypoints: { nodes: [], edges: [] },
    modes: [{ mode: 'hold', spot: 'Crossroads', radius_cells: 6, hold_ms: 120_000 }],
    tanks: defaultTanksForZones(w, h, entryZones, classes),
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
  if (changed) {
    markStudioEdited(state);
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
  if (filled > 0) {
    markStudioEdited(state);
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
  if (changed) {
    markStudioEdited(state);
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

export const MAP_WINDOW_SIDES = [256, 192, 160, 128, 96, 64] as const;
const MAX_WINDOWABLE_ROWS_BYTES = 64 * 1024;

export function windowCenteredOn(
  width: number,
  height: number,
  side: number,
  focusX: number,
  focusY: number
): [number, number, number, number] {
  const w = Math.max(1, Math.min(width, Math.floor(side)));
  const h = Math.max(1, Math.min(height, Math.floor(side)));
  const x0 = Math.max(0, Math.min(width - w, Math.floor(focusX) - Math.floor(w / 2)));
  const y0 = Math.max(0, Math.min(height - h, Math.floor(focusY) - Math.floor(h / 2)));
  return [x0, y0, w, h];
}

function normalizeTankDef(
  raw: Partial<EditorTankDef> & { label?: string; heading_deg?: number },
  idx: number
): EditorTankDef {
  const team: 'red' | 'blue' = raw.team === 'blue' ? 'blue' : 'red';
  const slot = (idx % 6) + 1;
  const headingMdeg =
    typeof raw.heading_mdeg === 'number'
      ? Math.round(((raw.heading_mdeg % 360000) + 360000) % 360000)
      : typeof raw.heading_deg === 'number'
        ? Math.round((((raw.heading_deg % 360) + 360) % 360) * 1000) % 360000
        : team === 'red'
          ? 90_000
          : 270_000;
  return {
    id: raw.id || `${team}_${slot}`,
    name: raw.name || raw.label || `${team === 'red' ? 'Red' : 'Blue'} Tank ${slot}`,
    team,
    x: Math.max(0, Math.floor(raw.x ?? 0)),
    y: Math.max(0, Math.floor(raw.y ?? 0)),
    heading_mdeg: headingMdeg,
    unit: raw.unit || 'tacsim.unit.line_tank'
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
    ...(state.modes.length > 0
      ? {
          modes: state.modes.map((m) => ({
            mode: 'hold' as const,
            spot: m.spot,
            radius_cells: m.radius_cells,
            hold_ms: m.hold_ms
          }))
        }
      : {}),
    ...(state.tanks.length > 0
      ? {
          tanks: state.tanks.map((t, i) => normalizeTankDef(t, i))
        }
      : {}),
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
    modes: (draft.modes ?? []).map((m) => ({
      mode: 'hold' as const,
      spot: m.spot,
      radius_cells: m.radius_cells,
      hold_ms: m.hold_ms
    })),
    tanks: (draft.tanks ?? []).map((t, i) => normalizeTankDef(t, i)),
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

const SPOT_LETTERS = [
  'A',
  'B',
  'C',
  'E',
  'F',
  'G',
  'H',
  'I',
  'J',
  'K',
  'L',
  'M',
  'N',
  'O',
  'P',
  'Q',
  'R',
  'S',
  'U',
  'V',
  'Y',
  'Z'
] as const;

function buildArenaMapWindow(
  state: EditorMapState,
  x0: number,
  y0: number,
  w: number,
  h: number
): Record<string, unknown> {
  const rows: [string, number][][] = [];
  const seenCodes: number[] = [];
  const winElev = new Int16Array(w * h);
  const stateW = state.width;
  const classes = state.classes;
  const elevCm = state.elevCm;

  for (let wy = 0; wy < h; wy++) {
    const y = y0 + wy;
    const rowRuns: [string, number][] = [];
    let currentGlyph = '';
    let runLen = 0;
    for (let wx = 0; wx < w; wx++) {
      const x = x0 + wx;
      const srcIdx = y * stateW + x;
      const code = classes[srcIdx] ?? 2;
      winElev[wy * w + wx] = elevCm[srcIdx] ?? 0;
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

  const spots = state.keySpots.slice(0, SPOT_LETTERS.length).map((s, i) => ({
    letter: SPOT_LETTERS[i]!,
    name: s.name,
    cell: [s.x, s.y]
  }));

  const isWindowed = x0 !== 0 || y0 !== 0 || w !== state.width || h !== state.height;
  const payload: Record<string, unknown> = {
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
    width: w,
    height: h,
    origin_cell: [x0, y0],
    rows,
    legend,
    spots,
    elev: encodeElevZigzagDelta(winElev)
  };
  if (isWindowed) {
    payload.extent = [state.width, state.height];
  }
  return payload;
}

function serializedMapFrameBytes(frame: Record<string, unknown>): number {
  const widest = { ...frame, match: 0 };
  // In Rust `WIDEST_MATCH` is `u64::MAX` (`18446744073709551615`, 20 chars vs `0`'s 1 char = +19 bytes).
  return new TextEncoder().encode(JSON.stringify(widest)).length + 19;
}

/**
 * Build live `arena.map` and `arena.cockpit` feeds from the current `EditorMapState`
 * and camera pin so `mush-pixel-wasm` renders the exact 1st-person, 2.5D, and overhead
 * views matching the server (`D-30-6`, `D-30-7`, `D-30-8`).
 *
 * When the map exceeds `256×256` (e.g. `512×512`) or when steep relief pushes the
 * unwindowed `arena.map` frame past `MAP_MAX_BYTES`, automatically windows `rows` and
 * `elev` around `pin` using `MAP_WINDOW_SIDES` with `origin_cell: [x0, y0]` and
 * `extent: [state.width, state.height]`.
 */
export function editorToPictureFeeds(
  state: EditorMapState,
  pin: EditorCameraPin
): { arenaMap: Record<string, unknown>; arenaCockpit: Record<string, unknown> } {
  const pinX = Math.max(0, Math.min(state.width - 1, Math.floor(pin.x)));
  const pinY = Math.max(0, Math.min(state.height - 1, Math.floor(pin.y)));
  const pinElev = state.elevCm[pinY * state.width + pinX] ?? 0;
  const headingMdeg = Math.round((((pin.headingDeg % 360) + 360) % 360) * 1000) % 360000;
  const xCm = Math.round((pinX + 0.5) * state.cellCm);
  const yCm = Math.round((pinY + 0.5) * state.cellCm);

  let arenaMap: Record<string, unknown> | null = null;
  if (state.width <= 256 && state.height <= 256) {
    const full = buildArenaMapWindow(state, 0, 0, state.width, state.height);
    const fullBytes = serializedMapFrameBytes(full);
    if (fullBytes <= MAP_MAX_BYTES) {
      arenaMap = full;
    } else {
      const rowsBytes = new TextEncoder().encode(JSON.stringify(full.rows)).length;
      if (rowsBytes > MAX_WINDOWABLE_ROWS_BYTES) {
        arenaMap = full;
      }
    }
  }
  if (!arenaMap) {
    for (const side of MAP_WINDOW_SIDES) {
      if (side > state.width && side > state.height) continue;
      const [x0, y0, w, h] = windowCenteredOn(state.width, state.height, side, pinX, pinY);
      const candidate = buildArenaMapWindow(state, x0, y0, w, h);
      if (serializedMapFrameBytes(candidate) <= MAP_MAX_BYTES) {
        arenaMap = candidate;
        break;
      }
    }
  }
  if (!arenaMap) {
    const [x0, y0, w, h] = windowCenteredOn(state.width, state.height, 64, pinX, pinY);
    arenaMap = buildArenaMapWindow(state, x0, y0, w, h);
  }

  return {
    arenaMap,
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
  const w = state.width;
  const h = state.height;
  const classes = state.classes;
  const x0 = Math.min(zone.x0, zone.x1);
  const x1 = Math.max(zone.x0, zone.x1);
  const y0 = Math.min(zone.y0, zone.y1);
  const y1 = Math.max(zone.y0, zone.y1);
  const passable = (x: number, y: number) =>
    x >= 0 &&
    x < w &&
    y >= 0 &&
    y < h &&
    moveCostForDrive(drive, classes[y * w + x] ?? 0) > 0;

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
  const classes = state.classes;
  const elevCm = state.elevCm;
  const cellCm = state.cellCm;
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
    const hereElev = elevCm[y * w + x] ?? 0;
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
      if (moveCostForDrive(drive, classes[nIdx] ?? 0) === 0) continue;
      if (steepnessPct(hereElev, elevCm[nIdx] ?? 0, false, cellCm) > limit) continue;
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
  const classes = state.classes;
  let min = 100;
  for (let i = 0; i < classes.length; i++) {
    const c = moveCostForDrive(drive, classes[i] ?? 0);
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
  const classes = state.classes;
  const elevCm = state.elevCm;
  const cellCm = state.cellCm;
  const [sx, sy] = start;
  const [gx, gy] = goal;
  if (sx < 0 || sx >= w || sy < 0 || sy >= h || gx < 0 || gx >= w || gy < 0 || gy >= h) {
    return null;
  }
  if (moveCostForDrive(drive, classes[sy * w + sx] ?? 0) === 0) return null;
  if (moveCostForDrive(drive, classes[gy * w + gx] ?? 0) === 0) return null;
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
    const hereElev = elevCm[node] ?? 0;

    for (const [dx, dy] of NEIGHBOURS_8) {
      const nx = x + dx;
      const ny = y + dy;
      if (nx < 0 || nx >= w || ny < 0 || ny >= h) continue;
      const nextIdx = ny * w + nx;
      const baseStep = moveCostForDrive(drive, classes[nextIdx] ?? 0);
      if (baseStep === 0) continue;

      const isDiag = dx !== 0 && dy !== 0;
      const thereElev = elevCm[nextIdx] ?? 0;
      const grade = slopePct(hereElev, thereElev, isDiag, cellCm);

      if (maxSlopePct !== null) {
        if (steepnessPct(hereElev, thereElev, isDiag, cellCm) > maxSlopePct) {
          continue;
        }
        if (isDiag) {
          const s1Idx = y * w + nx;
          const s2Idx = ny * w + x;
          const s1Pass = moveCostForDrive(drive, classes[s1Idx] ?? 0) > 0;
          const s2Pass = moveCostForDrive(drive, classes[s2Idx] ?? 0) > 0;
          const s1Elev = elevCm[s1Idx] ?? 0;
          const s2Elev = elevCm[s2Idx] ?? 0;
          const s1Steep =
            s1Pass &&
            (steepnessPct(hereElev, s1Elev, false, cellCm) > maxSlopePct ||
              steepnessPct(s1Elev, thereElev, false, cellCm) > maxSlopePct);
          const s2Steep =
            s2Pass &&
            (steepnessPct(hereElev, s2Elev, false, cellCm) > maxSlopePct ||
              steepnessPct(s2Elev, thereElev, false, cellCm) > maxSlopePct);
          if (s1Steep || s2Steep) continue;
          const s1Open = s1Pass && !s1Steep;
          const s2Open = s2Pass && !s2Steep;
          if (!s1Open && !s2Open) continue;
        }
      } else if (isDiag) {
        const s1Pass = moveCostForDrive(drive, classes[y * w + nx] ?? 0) > 0;
        const s2Pass = moveCostForDrive(drive, classes[ny * w + x] ?? 0) > 0;
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
  const feeds = editorToPictureFeeds(state, {
    x: Math.floor(state.width / 2),
    y: Math.floor(state.height / 2),
    headingDeg: 0
  });
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
  const classes = state.classes;
  const elevCm = state.elevCm;
  const cellCm = state.cellCm;

  const isLand = (code: number) => code !== 0 && code !== 4 && code !== 15 && code !== 10;
  let landTotal = 0;
  for (let i = 0; i < classes.length; i++) {
    if (isLand(classes[i] ?? 0)) landTotal++;
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
        if (flood[i] && isLand(classes[i] ?? 0)) reached++;
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
  let voidCount = 0;
  const w = state.width;
  const h = state.height;

  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const idx = y * w + x;
      const cHere = classes[idx] ?? 0;
      if (cHere === 0) voidCount++;
      const hereRoad = cHere === 7 || cHere === 1;
      const herePass = moveCostForDrive('tracked', cHere) > 0;
      const hereElev = elevCm[idx] ?? 0;

      for (const [nx, ny] of [
        [x + 1, y],
        [x, y + 1]
      ] as const) {
        if (nx >= w || ny >= h) continue;
        const nIdx = ny * w + nx;
        const cThere = classes[nIdx] ?? 0;
        const therePass = moveCostForDrive('tracked', cThere) > 0;
        const steep = steepnessPct(hereElev, elevCm[nIdx] ?? 0, false, cellCm);
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
      const hereElev = elevCm[sy * w + sx] ?? 0;
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
        const step = Math.abs(hereElev - (elevCm[ny * w + nx] ?? 0));
        if (step > maxEntryZoneStepCm) {
          maxEntryZoneStepCm = step;
        }
      }
    }
  }

  const failures: string[] = [];
  if (voidCount > 0) {
    failures.push(`grid contains ${voidCount} Void cell(s)`);
  }
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
  if (!zonesReachableWheeled) {
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
  const stepLimitCm = MAX_MUSTER_STEP_CM;
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
  markStudioEdited(state);
  return state.waypoints;
}

/**
 * Exact `mush_atlas::trace::sight_only` ray walk (`crates/mush-atlas/src/trace.rs:338-389`),
 * used by `computeViewshed` to match `tacsim::ground::viewshed` / `horizon.rs` cell-for-cell.
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
  const dx = Math.abs(hiX - loX);
  const dy = -Math.abs(hiY - loY);
  const n = Math.max(dx, -dy);
  if (n <= 1) {
    return 'clear';
  }

  // Exact mush_atlas::trace::Bresenham::new((loX, loY), (hiX, hiY))
  const sx = loX < hiX ? 1 : -1;
  const sy = loY < hiY ? 1 : -1;
  let err = dx + dy;
  let x = loX;
  let y = loY;
  let step = 0;
  let attenuation = 0;

  while (x !== hiX || y !== hiY) {
    const e2 = 2 * err;
    if (e2 >= dy) {
      err += dy;
      x += sx;
    }
    if (e2 <= dx) {
      err += dx;
      y += sy;
    }
    step += 1;
    if (x === hiX && y === hiY) break;

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
export function relaxEditorSlopes(
  state: EditorMapState,
  maxGradePct = maxSlopePctForDrive('tracked')
): number[] {
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
  const roadLimitPct = Math.max(0, maxSlopePctForDrive('wheeled') - 5);
  const roadMaxRise = Math.trunc((state.cellCm * roadLimitPct) / 100);
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
  if (changed.length > 0) {
    markStudioEdited(state);
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
  if (changed.length > 0) {
    markStudioEdited(state);
  }
  return changed;
}

// ---------------------------------------------------------------------------
// Power Features: Tank Roster Editor, Hold Objective, & Procedural Presets
// ---------------------------------------------------------------------------

/**
 * Re-muster all 12 stock tanks (`red_1..red_6`, `blue_1..blue_6`) onto distinct
 * passable cells inside their team's `entryZones` and level their 1-step muster
 * neighborhood to `<= 100 cm`.
 */
export function autoMusterTanks(state: EditorMapState): EditorTankDef[] {
  state.tanks = defaultTanksForZones(state.width, state.height, state.entryZones, state.classes);
  for (const t of state.tanks) {
    const baseElev = state.elevCm[t.y * state.width + t.x] ?? 0;
    for (const [dx, dy] of [
      [1, 0],
      [-1, 0],
      [0, 1],
      [0, -1]
    ] as const) {
      const nx = t.x + dx;
      const ny = t.y + dy;
      if (nx < 0 || nx >= state.width || ny < 0 || ny >= state.height) continue;
      const nIdx = ny * state.width + nx;
      const ne = state.elevCm[nIdx] ?? 0;
      if (ne - baseElev > MAX_MUSTER_STEP_CM) {
        state.elevCm[nIdx] = clampElevationCm(baseElev + MAX_MUSTER_STEP_CM);
      } else if (baseElev - ne > MAX_MUSTER_STEP_CM) {
        state.elevCm[nIdx] = clampElevationCm(baseElev - MAX_MUSTER_STEP_CM);
      }
    }
  }
  markStudioEdited(state);
  return state.tanks;
}

/**
 * Move tank slot `tankIndex` to `(x, y)`, clamping to its team's entry zone
 * (if present) and requiring a passable tracked cell not occupied by another tank.
 */
export function moveTankSlot(
  state: EditorMapState,
  tankIndex: number,
  x: number,
  y: number
): boolean {
  if (state.tanks.length === 0) {
    state.tanks = defaultTanksForZones(state.width, state.height, state.entryZones, state.classes);
  }
  const tank = state.tanks[tankIndex];
  if (!tank) return false;

  const zone = state.entryZones.find((z) => z.team === tank.team);
  const minX = zone ? Math.max(0, Math.min(zone.x0, zone.x1)) : 0;
  const maxX = zone ? Math.min(state.width - 1, Math.max(zone.x0, zone.x1)) : state.width - 1;
  const minY = zone ? Math.max(0, Math.min(zone.y0, zone.y1)) : 0;
  const maxY = zone ? Math.min(state.height - 1, Math.max(zone.y0, zone.y1)) : state.height - 1;

  const tx = Math.max(minX, Math.min(maxX, Math.floor(x)));
  const ty = Math.max(minY, Math.min(maxY, Math.floor(y)));
  if (tx === tank.x && ty === tank.y) return false;

  const code = state.classes[ty * state.width + tx] ?? 0;
  if (moveCostForDrive('tracked', code) === 0) return false;
  if (state.tanks.some((other, idx) => idx !== tankIndex && other.x === tx && other.y === ty)) {
    return false;
  }

  tank.x = tx;
  tank.y = ty;
  markStudioEdited(state);
  return true;
}

/**
 * Set the opening heading (degrees clockwise from north, `0..359`) of `tankIndex`.
 */
export function setTankHeadingDeg(
  state: EditorMapState,
  tankIndex: number,
  headingDeg: number
): boolean {
  if (state.tanks.length === 0) {
    state.tanks = defaultTanksForZones(state.width, state.height, state.entryZones, state.classes);
  }
  const tank = state.tanks[tankIndex];
  if (!tank) return false;
  const nextMdeg = Math.round((((headingDeg % 360) + 360) % 360) * 1000) % 360000;
  if (tank.heading_mdeg === nextMdeg) return false;
  tank.heading_mdeg = nextMdeg;
  markStudioEdited(state);
  return true;
}

/**
 * Return the active `hold` mode configuration from `state.modes`, or `null` if disabled.
 */
export function getHoldObjective(state: EditorMapState): EditorArenaMode | null {
  return state.modes.find((m) => m.mode === 'hold') ?? null;
}

/**
 * Enable, update, or disable the `hold` mode objective in `state.modes`.
 */
export function setHoldObjective(
  state: EditorMapState,
  config: {
    enabled: boolean;
    spot?: string;
    radius_cells?: number;
    hold_ms?: number;
  }
): EditorArenaMode | null {
  if (!config.enabled) {
    if (state.modes.length > 0) {
      state.modes = [];
      markStudioEdited(state);
    }
    return null;
  }
  const current = getHoldObjective(state);
  const fallbackSpot = state.keySpots[1]?.name ?? state.keySpots[0]?.name ?? 'The Crossroads';
  const spot = config.spot ?? current?.spot ?? fallbackSpot;
  const radiusCells = Math.max(
    1,
    Math.min(32, Math.round(config.radius_cells ?? current?.radius_cells ?? 6))
  );
  const holdMs = Math.max(
    10_000,
    Math.min(600_000, Math.round(config.hold_ms ?? current?.hold_ms ?? 120_000))
  );
  const nextMode: EditorArenaMode = {
    mode: 'hold',
    spot,
    radius_cells: radiusCells,
    hold_ms: holdMs
  };
  state.modes = [nextMode];
  markStudioEdited(state);
  return nextMode;
}

const PG_RIVER: [number, number] = [62, 66];
const PG_FORD: [number, number] = [92, 102];
const PG_ROAD_EW: [number, number] = [96, 98];
const PG_ROAD_NS: [number, number] = [128, 130];
const PG_BUILDINGS: readonly [number, number, number, number][] = [
  [150, 170, 36, 52],
  [96, 116, 186, 202],
  [200, 216, 140, 156]
];
const PG_WOODS_LIGHT: readonly [number, number, number, number][] = [
  [20, 58, 150, 200],
  [160, 210, 40, 80]
];
const PG_WOODS_HEAVY: readonly [number, number, number, number][] = [
  [30, 48, 165, 185],
  [175, 193, 50, 70]
];
const PG_RUBBLE: readonly [number, number, number, number][] = [[100, 112, 60, 72]];
const PG_BARE: readonly [number, number, number, number][] = [
  [210, 255, 0, 36],
  [0, 40, 0, 24]
];
const PG_BLUFF_TOP: [number, number, number, number] = [141, 147, 105, 118];
const PG_BLUFF_TOP_CM = 500;
const PG_BLUFF_RAMP: readonly [[number, number, number, number], number][] = [
  [[141, 146, 119, 119], 270],
  [[141, 146, 120, 120], 40]
];
const PG_V3_CORRIDOR: [number, number] = [86, 108];
const PG_V3_BLEND = 14.0;
const PG_V3_BLUFF_KEEP: [number, number, number, number] = [136, 152, 100, 125];
const PG_V3_BOWL: [number, number, number] = [88.0, 36.0, 18.0];
const PG_V3_SIGNAL_HILL: [number, number] = [185.0, 215.0];

function withinBox(x: number, y: number, b: readonly [number, number, number, number]): boolean {
  return x >= b[0] && x <= b[1] && y >= b[2] && y <= b[3];
}

function smoothstep01(t: number): number {
  const c = Math.max(0, Math.min(1, t));
  return c * c * (3 - 2 * c);
}

function sqNum(v: number): number {
  return v * v;
}

function gauss2d(dx: number, dy: number, sigma: number): number {
  return Math.exp(-(dx * dx + dy * dy) / (2 * sigma * sigma));
}

function boxDist(x: number, y: number, b: readonly [number, number, number, number]): number {
  const dx = Math.max(0, b[0] - x, x - b[1]);
  const dy = Math.max(0, b[2] - y, y - b[3]);
  return Math.max(dx, dy);
}

function pgClassAt256(x: number, y: number): TerrainClassCode {
  for (const b of PG_BUILDINGS) {
    if (withinBox(x, y, b)) {
      const doorX = Math.floor((b[0] + b[1]) / 2);
      if (y === b[2] && x === doorX) return 11; // door
      if (x === b[0] || x === b[1] || y === b[2] || y === b[3]) return 10; // wall
      return 5; // floor
    }
  }
  if (x >= PG_RIVER[0] && x <= PG_RIVER[1]) {
    return y >= PG_FORD[0] && y <= PG_FORD[1] ? 4 : 15; // water : water_deep
  }
  if ((y >= PG_ROAD_EW[0] && y <= PG_ROAD_EW[1]) || (x >= PG_ROAD_NS[0] && x <= PG_ROAD_NS[1])) {
    return 7; // road
  }
  for (const p of PG_WOODS_HEAVY) {
    if (withinBox(x, y, p)) return 14;
  }
  for (const p of PG_WOODS_LIGHT) {
    if (withinBox(x, y, p)) return 13;
  }
  for (const p of PG_RUBBLE) {
    if (withinBox(x, y, p)) return 6;
  }
  for (const p of PG_BARE) {
    if (withinBox(x, y, p)) return 2;
  }
  return 3; // grass
}

function pgV3Features(fx: number, fy: number): number {
  const highlands =
    3500 *
    smoothstep01((fy - 118) / 45) *
    (0.65 + 0.35 * Math.sin(fx / 19) * Math.cos(fy / 15));
  const signal = 8000 * gauss2d(fx - PG_V3_SIGNAL_HILL[0], fy - PG_V3_SIGNAL_HILL[1], 20);
  const knoll = 4500 * gauss2d(fx - 40, fy - 225, 16);
  const hills =
    3000 * gauss2d(fx - 205, fy - 128, 12) +
    3500 * gauss2d(fx - 28, fy - 58, 14) +
    4000 * gauss2d(fx - 228, fy - 188, 15);
  const [bx, by, rim] = PG_V3_BOWL;
  const r = Math.hypot(fx - bx, fy - by);
  const bowl =
    r < rim ? -1200 + 3200 * (r / rim) * (r / rim) : 2000 * Math.exp(-sqNum((r - rim) / 9));
  const valley = -1200 * Math.exp(-sqNum((fx - 64) / 16));
  const spine =
    2000 *
    Math.exp(-sqNum((fy - 66) / 6)) *
    smoothstep01((fx - 180) / 12) *
    smoothstep01((240 - fx) / 12);
  const shelf =
    2500 *
    smoothstep01((fx - 206) / 12) *
    smoothstep01((254 - fx) / 12) *
    smoothstep01((fy + 2) / 12) *
    smoothstep01((36 - fy) / 12);
  return highlands + signal + knoll + hills + bowl + valley + spine + shelf;
}

function pgElevV3At256(x: number, y: number): number {
  if (withinBox(x, y, PG_BLUFF_TOP)) return PG_BLUFF_TOP_CM;
  for (const [b, rampCm] of PG_BLUFF_RAMP) {
    if (withinBox(x, y, b)) return rampCm;
  }
  const raw =
    200 * Math.sin(x / 40) + 150 * Math.cos(y / 55) + 100 * Math.sin((x + y) / 70);
  let base = Math.round(raw / 100) * 100;
  const cls = pgClassAt256(x, y);
  if (cls === 4 || cls === 15) base -= 200;

  const corridor = Math.max(0, PG_V3_CORRIDOR[0] - y, y - PG_V3_CORRIDOR[1]);
  const road = Math.abs(x - (PG_ROAD_NS[0] + 1)) - 1;
  const keep = Math.min(corridor, Math.max(0, road), boxDist(x, y, PG_V3_BLUFF_KEEP));
  const w = smoothstep01(keep / PG_V3_BLEND);
  let feature = pgV3Features(x, y);
  for (const b of PG_BUILDINGS) {
    const near = boxDist(x, y, [Math.max(0, b[0] - 2), b[1] + 2, Math.max(0, b[2] - 2), b[3] + 2]);
    const pad = pgV3Features((b[0] + b[1]) / 2, (b[2] + b[3]) / 2);
    const t = smoothstep01(near / 6);
    feature = pad + (feature - pad) * t;
  }
  return clampElevationCm(Math.round((base + w * feature) / 10) * 10);
}

/**
 * Procedurally generate a complete 256×256 `EditorMapState` for any of the 4 doc 30
 * generator presets (`proving_ground`, `mars_canyon`, `lunar_crater`, `ice_shelf`),
 * matching `tacsim::ground::generate` (doc 30 §4, §9, D-30-9).
 */
export function generatePresetMap(
  preset: GeneratorPresetName,
  seed = 2350
): EditorMapState {
  const SIDE = 256;
  const classes = new Uint8Array(SIDE * SIDE);
  const elevCm = new Int16Array(SIDE * SIDE);

  const entryZones: EditorEntryZone[] = [
    { team: 'red', x0: 0, y0: 0, x1: 15, y1: 255 },
    { team: 'blue', x0: 240, y0: 0, x1: 255, y1: 255 }
  ];
  const keySpots: EditorKeySpot[] = [
    { name: 'The Ford', x: 64, y: 97 },
    { name: 'The Crossroads', x: 129, y: 97 },
    { name: 'North Barn', x: 106, y: 194 },
    { name: 'East Compound', x: 208, y: 148 },
    { name: 'The Bluff', x: 144, y: 111 }
  ];
  const waypoints: EditorWaypointGraph = {
    nodes: [
      { id: 'ew_008', x: 8, y: 97 },
      { id: 'ew_032', x: 32, y: 97 },
      { id: 'ew_056', x: 56, y: 97 },
      { id: 'ew_064', x: 64, y: 97 },
      { id: 'ew_080', x: 80, y: 97 },
      { id: 'ew_104', x: 104, y: 97 },
      { id: 'xroads', x: 129, y: 97 },
      { id: 'ew_152', x: 152, y: 97 },
      { id: 'ew_176', x: 176, y: 97 },
      { id: 'ew_200', x: 200, y: 97 },
      { id: 'ew_224', x: 224, y: 97 },
      { id: 'ew_248', x: 248, y: 97 },
      { id: 'ns_008', x: 129, y: 8 },
      { id: 'ns_032', x: 129, y: 32 },
      { id: 'ns_056', x: 129, y: 56 },
      { id: 'ns_080', x: 129, y: 80 },
      { id: 'ns_120', x: 129, y: 120 },
      { id: 'ns_144', x: 129, y: 144 },
      { id: 'ns_168', x: 129, y: 168 },
      { id: 'ns_192', x: 129, y: 192 },
      { id: 'ns_216', x: 129, y: 216 },
      { id: 'ns_248', x: 129, y: 248 },
      { id: 'barn', x: 106, y: 194 },
      { id: 'compound', x: 208, y: 148 },
      { id: 'muster_red', x: 6, y: 105 },
      { id: 'muster_blue', x: 249, y: 105 }
    ],
    edges: [
      ['ew_008', 'ew_032'],
      ['ew_032', 'ew_056'],
      ['ew_056', 'ew_064'],
      ['ew_064', 'ew_080'],
      ['ew_080', 'ew_104'],
      ['ew_104', 'xroads'],
      ['xroads', 'ew_152'],
      ['ew_152', 'ew_176'],
      ['ew_176', 'ew_200'],
      ['ew_200', 'ew_224'],
      ['ew_224', 'ew_248'],
      ['ns_008', 'ns_032'],
      ['ns_032', 'ns_056'],
      ['ns_056', 'ns_080'],
      ['ns_080', 'xroads'],
      ['xroads', 'ns_120'],
      ['ns_120', 'ns_144'],
      ['ns_144', 'ns_168'],
      ['ns_168', 'ns_192'],
      ['ns_192', 'ns_216'],
      ['ns_216', 'ns_248'],
      ['ns_192', 'barn'],
      ['ew_200', 'compound'],
      ['ew_008', 'muster_red'],
      ['ew_248', 'muster_blue']
    ]
  };
  const modes: EditorArenaMode[] = [
    { mode: 'hold', spot: 'The Crossroads', radius_cells: 6, hold_ms: 120_000 }
  ];
  const tanks: EditorTankDef[] = [
    { id: 'red_1', name: 'Red Tank 1', team: 'red', x: 6, y: 97, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'red_2', name: 'Red Tank 2', team: 'red', x: 6, y: 95, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'red_3', name: 'Red Tank 3', team: 'red', x: 6, y: 99, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'red_4', name: 'Red Tank 4', team: 'red', x: 6, y: 93, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_1', name: 'Blue Tank 1', team: 'blue', x: 249, y: 97, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_2', name: 'Blue Tank 2', team: 'blue', x: 249, y: 95, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_3', name: 'Blue Tank 3', team: 'blue', x: 249, y: 99, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_4', name: 'Blue Tank 4', team: 'blue', x: 249, y: 93, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' },
    { id: 'red_5', name: 'Red Tank 5', team: 'red', x: 6, y: 101, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'red_6', name: 'Red Tank 6', team: 'red', x: 4, y: 94, heading_mdeg: 90_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_5', name: 'Blue Tank 5', team: 'blue', x: 249, y: 101, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' },
    { id: 'blue_6', name: 'Blue Tank 6', team: 'blue', x: 251, y: 94, heading_mdeg: 270_000, unit: 'tacsim.unit.line_tank' }
  ];

  if (preset === 'proving_ground') {
    for (let y = 0; y < SIDE; y++) {
      for (let x = 0; x < SIDE; x++) {
        const idx = y * SIDE + x;
        classes[idx] = pgClassAt256(x, y);
        elevCm[idx] = pgElevV3At256(x, y);
      }
    }
    return {
      id: 'proving_ground',
      label: 'The Proving Ground',
      biome: 'temperate',
      body: 'luna',
      width: SIDE,
      height: SIDE,
      cellCm: 1000,
      originM: [20_000, 20_000],
      classes,
      elevCm,
      keySpots,
      entryZones,
      waypoints,
      modes,
      tanks,
      atlasVersion: 3,
      arenaVersion: 5,
      provenance: {
        source: 'generator',
        version: 'v3',
        seed: 2350
      },
      note: 'Generated proving_ground (v3, seed 2350)'
    };
  }

  const meta = GENERATOR_PRESETS.find((p) => p.id === preset)!;
  const biome = meta.biome;
  const cleanSeed = Math.max(0, Math.floor(seed));
  const phase = (cleanSeed % 10_000) * 0.017_453_292_519_943_295;
  const shiftX = 6.0 * Math.sin(phase);
  const shiftY = 6.0 * Math.cos(phase * 1.3);
  const roadClass: TerrainClassCode = biome === 'lunar' ? 1 : 7;

  for (let y = 0; y < SIDE; y++) {
    for (let x = 0; x < SIDE; x++) {
      const idx = y * SIDE + x;
      let cls: TerrainClassCode;
      if (withinBox(x, y, PG_BLUFF_TOP)) {
        cls = biome === 'mars' ? 8 : biome === 'lunar' ? 17 : 16;
      } else if (PG_BLUFF_RAMP.some(([b]) => withinBox(x, y, b))) {
        cls = 2;
      } else {
        const bldg = PG_BUILDINGS.find((b) => withinBox(x, y, b));
        if (bldg) {
          const doorX = Math.floor((bldg[0] + bldg[1]) / 2);
          if (y === bldg[2] && x === doorX) cls = 11;
          else if (x === bldg[0] || x === bldg[1] || y === bldg[2] || y === bldg[3]) cls = 10;
          else cls = 5;
        } else if (
          (y >= PG_ROAD_EW[0] && y <= PG_ROAD_EW[1]) ||
          (x >= PG_ROAD_NS[0] && x <= PG_ROAD_NS[1]) ||
          (x >= 106 && x <= 128 && y >= 184 && y <= 185) ||
          (x >= 207 && x <= 209 && y >= 99 && y <= 139)
        ) {
          cls = roadClass;
        } else if (x >= PG_RIVER[0] && x <= PG_RIVER[1]) {
          if (y >= PG_FORD[0] && y <= PG_FORD[1]) {
            cls = biome === 'ice' ? 16 : 2;
          } else {
            cls = biome === 'ice' ? 15 : 2;
          }
        } else {
          const n =
            Math.sin((x + shiftX) * 0.09 + phase) * Math.cos((y + shiftY) * 0.08 - phase * 0.5) +
            0.5 * Math.sin((x - y) * 0.05 + phase);
          const distRoad = Math.min(Math.abs(x - 129), Math.abs(y - 97));
          const inEntry = x <= 18 || x >= 237;
          if (!inEntry && distRoad >= 8 && withinBox(x, y, PG_RUBBLE[0]!)) {
            cls = 6;
          } else if (
            !inEntry &&
            distRoad >= 10 &&
            (withinBox(x, y, [24, 52, 156, 192]) || withinBox(x, y, [168, 202, 46, 74]))
          ) {
            cls = n > 0.35 ? 6 : 9;
          } else if (biome === 'mars') {
            cls = n > 0.45 ? 17 : n < -0.35 ? 2 : 8;
          } else if (biome === 'lunar') {
            cls = n > 0.3 ? 2 : 17;
          } else {
            cls = n > 0.35 ? 2 : 16;
          }
        }
      }
      classes[idx] = cls;

      if (withinBox(x, y, PG_BLUFF_TOP)) {
        elevCm[idx] = PG_BLUFF_TOP_CM;
        continue;
      }
      const rampHit = PG_BLUFF_RAMP.find(([b]) => withinBox(x, y, b));
      if (rampHit) {
        elevCm[idx] = rampHit[1];
        continue;
      }

      const edgeDist = Math.min(Math.max(0, x - 15), Math.max(0, 240 - x));
      const edgeW = smoothstep01(edgeDist / 16);
      const corridor = Math.max(0, PG_V3_CORRIDOR[0] - y, y - PG_V3_CORRIDOR[1]);
      const roadNs = Math.abs(x - (PG_ROAD_NS[0] + 1)) - 1;
      const spurBarn = boxDist(x, y, [104, 130, 182, 204]);
      const spurComp = boxDist(x, y, [198, 218, 96, 158]);
      const keep = Math.min(
        corridor,
        Math.max(0, roadNs),
        spurBarn,
        spurComp,
        boxDist(x, y, PG_V3_BLUFF_KEEP)
      );
      const w = smoothstep01(keep / PG_V3_BLEND) * edgeW;
      const baseWave =
        edgeW * (160 * Math.sin((x + shiftX) / 42) + 120 * Math.cos((y + shiftY) / 52));

      let feature: number;
      if (biome === 'mars') {
        const wadi = -450 * Math.exp(-sqNum((x - 64) / 11));
        const northShelf = 3200 * smoothstep01((y - 122) / 36);
        const southShelf = 2200 * smoothstep01((72 - y) / 28);
        const spurPeak = 6200 * gauss2d(x - (184 + shiftX * 0.5), y - (214 + shiftY * 0.5), 22);
        const craterR = Math.hypot(x - 88, y - 38);
        const crater =
          craterR < 16
            ? -900 + 2200 * sqNum(craterR / 16)
            : 1300 * Math.exp(-sqNum((craterR - 16) / 8));
        feature = wadi + northShelf + southShelf + spurPeak + crater;
      } else if (biome === 'lunar') {
        const r1 = Math.hypot(x - (86 + shiftX * 0.4), y - (40 + shiftY * 0.4));
        const c1 =
          r1 < 18 ? -1100 + 2900 * sqNum(r1 / 18) : 1800 * Math.exp(-sqNum((r1 - 18) / 8.5));
        const r2 = Math.hypot(x - (182 - shiftX * 0.4), y - (210 - shiftY * 0.4));
        const c2 =
          r2 < 22
            ? -1200 + 3600 * sqNum(r2 / 22) + 1400 * gauss2d(x - 182, y - 210, 5)
            : 2400 * Math.exp(-sqNum((r2 - 22) / 9.5));
        const highlands = 2800 * smoothstep01((y - 135) / 42);
        feature = c1 + c2 + highlands;
      } else {
        const shelf = 2900 * smoothstep01((y - 125) / 40);
        const lead = -380 * Math.exp(-sqNum((x - 64) / 10));
        const ridgeA =
          2400 *
          Math.exp(-sqNum((y - 58 - 5 * Math.sin(x / 24)) / 9)) *
          smoothstep01((x - 30) / 14) *
          smoothstep01((225 - x) / 14);
        const dome = 5200 * gauss2d(x - (186 + shiftX * 0.4), y - (214 + shiftY * 0.4), 21);
        feature = shelf + lead + ridgeA + dome;
      }

      for (const b of PG_BUILDINGS) {
        const near = boxDist(x, y, [
          Math.max(0, b[0] - 2),
          b[1] + 2,
          Math.max(0, b[2] - 2),
          b[3] + 2
        ]);
        feature *= smoothstep01(near / 6);
      }
      elevCm[idx] =
        x <= 15 || x >= 240 ? 0 : clampElevationCm(Math.round((baseWave + w * feature) / 10) * 10);
    }
  }

  const originM: [number, number] =
    biome === 'mars' ? [40_000, 40_000] : biome === 'lunar' ? [50_000, 50_000] : [60_000, 60_000];

  const state: EditorMapState = {
    id: preset,
    label:
      preset === 'mars_canyon'
        ? 'Mars Canyon'
        : preset === 'lunar_crater'
          ? 'Lunar Crater'
          : 'Ice Shelf',
    biome,
    body: meta.body,
    width: SIDE,
    height: SIDE,
    cellCm: 1000,
    originM,
    classes,
    elevCm,
    keySpots,
    entryZones,
    waypoints,
    modes,
    tanks,
    atlasVersion: 1,
    arenaVersion: 1,
    provenance: {
      source: 'generator',
      version: 'v1',
      seed: cleanSeed
    },
    note: ''
  };
  despeckleEditorMap(state, 3);
  state.provenance = {
    source: 'generator',
    version: 'v1',
    seed: cleanSeed
  };
  return state;
}
