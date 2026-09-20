// The tactical map (doc 14 §6, plan §3.5): the latest `atlas.map` keyframe
// — a window as a grid of glyph cells, its legend and the units the server
// let this viewer see — and the deltas that move the units on it. Cells
// change only with a new keyframe (a new composite revision); a delta
// carries the units and a line to say what happened. Read defensively at
// the JSON boundary; the wire types in ./proto are the contract, and the
// text renderer (mush-ansi) is the acceptance test of every frame (P10).

import type { AtlasMapFrame, MapUnit } from "../proto";

/** The `atlas.map` feed id. */
export const MAP_FEED = "atlas.map";

/**
 * **The terrain palette**, one colour a glyph (`mush_atlas::TerrainClass::glyph`
 * is the whole list). The glyph is the fact and the colour is only how it is
 * painted: nothing here knows a rule, and an unpainted glyph is the void.
 *
 * One table, two canvases (16b cut Q.5): the atlas map and the arena's cockpit
 * map draw the same ground from the same server-side vocabulary, so a second
 * copy would be a second answer to "what colour is a wood". The five classes
 * cut Q.5 added — light and heavy woods, deep water, ice, regolith — used to
 * fall through to the void on the atlas canvas, which drew nothing at all for a
 * cell that has terrain.
 */
export const TERRAIN_COLOURS: Readonly<Record<string, string>> = {
  " ": "transparent",
  ".": "#6b6b6b",
  ",": "#7a5a3a",
  '"': "#3f7a3a",
  "~": "#2a5c8a",
  _: "#a89a7a",
  x: "#8a6a4a",
  "=": "#4a4a4a",
  ":": "#c2a86b",
  "^": "#7d7d7d",
  "#": "#1f1f1f",
  "+": "#c78a3a",
  D: "#8c7b5a",
  t: "#2f5f33",
  T: "#1f4423",
  W: "#173d5e",
  i: "#9fc4d8",
  r: "#6f6558",
};

/** The most notes kept. */
export const MAP_NOTES = 20;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function cell(raw: unknown): [number, number] | null {
  return Array.isArray(raw) &&
    raw.length === 2 &&
    typeof raw[0] === "number" &&
    typeof raw[1] === "number" &&
    raw[0] >= 0 &&
    raw[1] >= 0
    ? [raw[0], raw[1]]
    : null;
}

function parseUnit(raw: unknown): MapUnit | null {
  if (!isObj(raw) || typeof raw.id !== "string" || typeof raw.name !== "string") {
    return null;
  }
  const at = cell(raw.cell);
  if (!at) {
    return null;
  }
  return {
    id: raw.id,
    name: raw.name,
    cell: at,
    glyph: typeof raw.glyph === "string" && raw.glyph.length > 0 ? raw.glyph : "o",
    you: raw.you === true,
  };
}

/** Parse the units of a frame or delta. */
export function parseUnits(raw: unknown): MapUnit[] {
  if (!Array.isArray(raw)) {
    return [];
  }
  const out: MapUnit[] = [];
  for (const u of raw) {
    const unit = parseUnit(u);
    if (unit) {
      out.push(unit);
    }
  }
  return out;
}

/** Parse an `atlas.map` keyframe; `null` when it is not one. */
export function parseMapFrame(raw: unknown): AtlasMapFrame | null {
  if (!isObj(raw) || !isObj(raw.schema) || raw.schema.shape !== "grid") {
    return null;
  }
  if (
    typeof raw.window !== "number" ||
    typeof raw.width !== "number" ||
    typeof raw.height !== "number" ||
    !Array.isArray(raw.rows)
  ) {
    return null;
  }
  const rows: [string, number][][] = raw.rows.map((row) =>
    Array.isArray(row)
      ? row
          .map((run): [string, number] | null =>
            Array.isArray(run) && typeof run[0] === "string" && typeof run[1] === "number"
              ? [run[0], run[1]]
              : null,
          )
          .filter((run): run is [string, number] => run !== null)
      : [],
  );
  const legend: [string, string][] = Array.isArray(raw.legend)
    ? raw.legend
        .map((e): [string, string] | null =>
          Array.isArray(e) && typeof e[0] === "string" && typeof e[1] === "string"
            ? [e[0], e[1]]
            : null,
        )
        .filter((e): e is [string, string] => e !== null)
    : [];
  const origin = Array.isArray(raw.origin) && raw.origin.length === 2
    ? ([Number(raw.origin[0]) || 0, Number(raw.origin[1]) || 0] as [number, number])
    : ([0, 0] as [number, number]);
  return {
    schema: {
      id: typeof raw.schema.id === "string" ? raw.schema.id : MAP_FEED,
      version: typeof raw.schema.version === "number" ? raw.schema.version : 1,
      shape: "grid",
    },
    window: raw.window,
    revision: typeof raw.revision === "number" ? raw.revision : 0,
    tick: typeof raw.tick === "number" ? raw.tick : 0,
    title: typeof raw.title === "string" ? raw.title : "",
    surface: typeof raw.surface === "string" ? raw.surface : "",
    floors: Array.isArray(raw.floors) ? raw.floors.filter((f): f is string => typeof f === "string") : [],
    origin,
    cell_cm: typeof raw.cell_cm === "number" ? raw.cell_cm : 100,
    width: raw.width,
    height: raw.height,
    rows,
    legend,
    units: parseUnits(raw.units),
    you: cell(raw.you),
  };
}

/** The grid's rows as glyph arrays, row 0 the south edge, each exactly
 * `width` long, with the units overlaid — the web's copy of the renderer's
 * `expand_rows`, so the canvas and the text agree cell for cell. */
export function expandRows(frame: AtlasMapFrame): string[][] {
  const rows: string[][] = [];
  for (let y = 0; y < frame.height; y += 1) {
    const row: string[] = [];
    for (const [glyph, n] of frame.rows[y] ?? []) {
      const g = glyph.charAt(0) || " ";
      for (let i = 0; i < n && row.length < frame.width; i += 1) {
        row.push(g);
      }
    }
    while (row.length < frame.width) {
      row.push(" ");
    }
    rows.push(row);
  }
  for (const unit of frame.units) {
    const [x, y] = unit.cell;
    const row = rows[y];
    if (row && x < frame.width) {
      row[x] = unit.glyph.charAt(0) || "o";
    }
  }
  return rows;
}

export class MapStore {
  frame = $state<AtlasMapFrame | null>(null);
  notes = $state<string[]>([]);

  /** Whether a map is held. */
  get loaded(): boolean {
    return this.frame !== null;
  }

  /** A keyframe replaces the whole map; `false` when the payload is not one. */
  applyKeyframe(raw: unknown): boolean {
    const frame = parseMapFrame(raw);
    if (!frame) {
      return false;
    }
    this.frame = frame;
    return true;
  }

  /** A delta moves the units and adds its note; one for another window, or
   * for no map, is ignored. */
  applyDelta(raw: unknown): void {
    if (!isObj(raw) || !this.frame || raw.window !== this.frame.window) {
      return;
    }
    if (Array.isArray(raw.units)) {
      this.frame = { ...this.frame, units: parseUnits(raw.units), you: cell(raw.you) };
    }
    if (typeof raw.note === "string" && raw.note.length > 0) {
      this.notes = [...this.notes.slice(-(MAP_NOTES - 1)), raw.note];
    }
  }

  clear(): void {
    this.frame = null;
    this.notes = [];
  }
}
