// The quarry on the web (doc 20 cut SM.9; §10, D-20-13): the wire types of the `site.map` and
// `body.ground` panels, their decoders, and the pure helpers `QuarryScreen.svelte` paints from —
// the rig prefills, the view/layer/rig steps (the TUI's own rules), a clicked cell's site cell,
// a site's point on the 2:1 body picture, the leg rule that places a moving rig, and one colour
// per glyph.
//
// One protocol, three renderers (D-20-13): both frames are GRID frames (`schema.shape ==
// "grid"`, row 0 the south edge) read through the generic grid decoder (`parseMapFrame` /
// `expandRows`), with a `site` or `ground` vector block beside the grid. **No helper here
// computes a glyph**: every glyph is the frame's; a colour or a shape for a glyph is rendering.
// The one thing a client computes is where along a frozen leg the viewer's own rig stands now
// (the lead's 0z ruling 1) — `legPosition`, the server's `leg_position` rule, pinned to the
// same expectations — and the glyph it restores behind a moving rig is the frame's own
// (`site.under`).
//
// Everything is pure and DOM-free, so vitest pins it. The payloads are `serde_json::Value`
// on the server — outside ts-rs — so these types are hand-written and the decoders are
// tolerant of a missing key and of an unknown one.

import { expandRows, parseMapFrame } from "./stores/map.svelte";
import type { AtlasMapFrame, JsonValue } from "./proto";

/** The site map feed (`tacsim::port::site_map::SITE_MAP_FEED`). */
export const SITE_MAP_FEED = "site.map";
/** The ground chart feed (`tacsim::port::ground_chart::BODY_GROUND_FEED`). */
export const BODY_GROUND_FEED = "body.ground";
/** The pushed status feed (`tacsim::port::site_map::TV_STATUS_FEED`): the cue to re-pull. */
export const TV_STATUS_FEED = "tv.status";

/** The quarry's three views (`z` steps drive → site → ground → drive). */
export type QuarryMode = "drive" | "site" | "ground";

/** One waypoint of a frozen leg, site centimetres and seconds from its start. */
export interface Waypoint {
  x_cm: number;
  y_cm: number;
  at_s: number;
}

/** One of the viewer's readings rows. */
export interface SiteReading {
  text: string;
  cell: [number, number];
  m: number;
  dir: string;
  /** The pocket address or field name the row reads (a drive target). */
  target: string;
  /** The row's words, as the readings line joins them. */
  line: string;
}

/** A rig's run, from its operator's own projection (never its truth). */
export interface SiteRun {
  addr: string;
  ore: string;
  k: number;
  cycles: number;
  due_ic: number;
  stop: boolean;
  blind: boolean;
  text: string;
}

/** A rig's fill plan as its author's frame carries it (doc 20 cut SM.11): step words and hours only. */
export interface SitePlan {
  no: number;
  /** `live`, `parking` or `ended`. */
  state: string;
  step: number;
  steps: number;
  /** Up to four step words from the cursor. */
  next: string[];
  deadline_ic: number;
  end: string | null;
  /** The plan's line under the rig. */
  text: string;
}

/** One of the viewer's rigs. */
export interface SiteRig {
  slot: number;
  mark: string;
  cell: [number, number] | null;
  charge_pct: number | null;
  hopper: string;
  act: { kind: string; due_ic: number } | null;
  leg: { started_ic: number; waypoints: Waypoint[] } | null;
  /** The rig's line exactly as `+tv/map` prints it. */
  line: string;
  run: SiteRun | null;
  /** The queued unload's line, when one is queued. */
  queued: string | null;
  /** The rig's fill plan, on its author's frame only (doc 20 cut SM.11). */
  plan: SitePlan | null;
}

/** The `site` vector block. */
export interface SiteBlock {
  view: string;
  site: string;
  family: string;
  family_word: string;
  families: string[];
  readings: SiteReading[];
  rigs: SiteRig[];
  hold: { room_t: number; line: string } | null;
  lines: string[];
  print: string[];
  addressed: number | null;
  grid: { x0: number; y0: number; span: number; cell_cm: number } | null;
  marks: {
    letters: [number, number, string][];
    hull: [number, number] | null;
    slots: [number, number, number][];
  };
  under: [number, number, string][];
  clock: { now_ic: number; rate: number } | null;
  log: { at_ic: number; text: string }[];
  /** The no-readings sentence whole (`MAP_NO_READINGS`), `null` with readings in view. */
  no_readings: string | null;
  /** The other-rigs sentence whole and unwrapped, `null` with none in view (`lines` holds it
   * wrapped for line mode). */
  other_rigs: string | null;
  /** Doc 32 cut CF.4: the presence sentence whole (`Presence: …`), `null` with no other flight
   * on the site — the flights beyond sight, which `other_rigs` does not name. */
  presence_line: string | null;
}

/** A decoded `site.map` frame: the grid, and the block (`null` on a reason frame). */
export interface SiteFrame {
  grid: AtlasMapFrame;
  site: SiteBlock | null;
}

/** One site of the ground chart. */
export interface GroundSite {
  letter: string;
  site: string;
  entry: string;
  venue: string;
  cell: [number, number];
  drawn: boolean;
  lat_mdeg: number;
  lon_mdeg: number;
  survey: string | null;
  tenure: string;
  passage: string | null;
  here: boolean;
  best: string | null;
  dark: boolean;
  /** The list's own sentence for the viewer's readings there (none, barren, dark or the best):
   * what a screen shows, never words of its own. */
  best_line: string;
}

/** One port of the ground chart. */
export interface GroundPort {
  glyph: string;
  port: string;
  cell: [number, number];
  drawn: boolean;
  lat_mdeg: number;
  lon_mdeg: number;
}

/** The `ground` vector block. */
export interface GroundBlock {
  body: string;
  name: string;
  family: string;
  family_word: string;
  families: string[];
  ports: GroundPort[];
  sites: GroundSite[];
  also: string[];
  footer: string;
  lines: string[];
  print: string[];
}

/** A decoded `body.ground` frame: the grid, and the block (`null` on a reason frame). */
export interface GroundFrame {
  grid: AtlasMapFrame;
  ground: GroundBlock | null;
}

type Obj = Record<string, unknown>;

function isObj(v: unknown): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function str(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function numOrNull(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function strs(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((s): s is string => typeof s === "string") : [];
}

function pair(v: unknown): [number, number] | null {
  return Array.isArray(v) && typeof v[0] === "number" && typeof v[1] === "number" ? [v[0], v[1]] : null;
}

function triples<T>(v: unknown, third: (x: unknown) => T | null): [number, number, T][] {
  if (!Array.isArray(v)) {
    return [];
  }
  const out: [number, number, T][] = [];
  for (const e of v) {
    if (Array.isArray(e) && typeof e[0] === "number" && typeof e[1] === "number") {
      const t = third(e[2]);
      if (t !== null) {
        out.push([e[0], e[1], t]);
      }
    }
  }
  return out;
}

function waypoints(v: unknown): Waypoint[] {
  return Array.isArray(v)
    ? v.filter(isObj).map((w) => ({ x_cm: num(w.x_cm), y_cm: num(w.y_cm), at_s: num(w.at_s) }))
    : [];
}

function rig(v: unknown): SiteRig | null {
  if (!isObj(v) || typeof v.slot !== "number") {
    return null;
  }
  const run = isObj(v.run)
    ? {
        addr: str(v.run.addr),
        ore: str(v.run.ore),
        k: num(v.run.k),
        cycles: num(v.run.cycles),
        due_ic: num(v.run.due_ic),
        stop: v.run.stop === true,
        blind: v.run.blind === true,
        text: str(v.run.text),
      }
    : null;
  return {
    slot: v.slot,
    mark: str(v.mark),
    cell: pair(v.cell),
    charge_pct: numOrNull(v.charge_pct),
    hopper: str(v.hopper),
    act: isObj(v.act) ? { kind: str(v.act.kind), due_ic: num(v.act.due_ic) } : null,
    leg: isObj(v.leg) ? { started_ic: num(v.leg.started_ic), waypoints: waypoints(v.leg.waypoints) } : null,
    line: str(v.line),
    run,
    queued: typeof v.queued === "string" ? v.queued : null,
    plan: isObj(v.plan)
      ? {
          no: num(v.plan.no),
          state: str(v.plan.state),
          step: num(v.plan.step),
          steps: num(v.plan.steps),
          next: strs(v.plan.next),
          deadline_ic: num(v.plan.deadline_ic),
          end: typeof v.plan.end === "string" ? v.plan.end : null,
          text: str(v.plan.text),
        }
      : null,
  };
}

/** **Decode a `site.map` frame** (every SM.9 key optional): `null` when it is no grid frame. */
export function parseSiteFrame(raw: unknown): SiteFrame | null {
  const grid = parseMapFrame(raw);
  if (!grid || !isObj(raw)) {
    return null;
  }
  const s = raw.site;
  if (!isObj(s) || grid.width === 0) {
    return { grid, site: null };
  }
  const readings: SiteReading[] = Array.isArray(s.readings)
    ? s.readings.filter(isObj).map((r) => ({
        text: str(r.text),
        cell: pair(r.cell) ?? [0, 0],
        m: num(r.m),
        dir: str(r.dir),
        target: str(r.target),
        line: str(r.line, str(r.text)),
      }))
    : [];
  const marks = isObj(s.marks) ? s.marks : {};
  return {
    grid,
    site: {
      view: str(s.view),
      site: str(s.site),
      family: str(s.family),
      family_word: str(s.family_word),
      families: strs(s.families),
      readings,
      rigs: Array.isArray(s.rigs) ? s.rigs.map(rig).filter((r): r is SiteRig => r !== null) : [],
      hold: isObj(s.hold) ? { room_t: num(s.hold.room_t), line: str(s.hold.line) } : null,
      lines: strs(s.lines),
      print: strs(s.print),
      addressed: numOrNull(s.addressed),
      grid: isObj(s.grid)
        ? {
            x0: num(s.grid.x0),
            y0: num(s.grid.y0),
            span: num(s.grid.span, 1) || 1,
            cell_cm: num(s.grid.cell_cm, 1000) || 1000,
          }
        : null,
      marks: {
        letters: triples(marks.letters, (x) => (typeof x === "string" ? x : null)),
        hull: pair(marks.hull),
        slots: triples(marks.slots, (x) => (typeof x === "number" ? x : null)),
      },
      under: triples(s.under, (x) => (typeof x === "string" ? x : null)),
      clock: isObj(s.clock) ? { now_ic: num(s.clock.now_ic), rate: num(s.clock.rate) } : null,
      log: Array.isArray(s.log)
        ? s.log.filter(isObj).map((l) => ({ at_ic: num(l.at_ic), text: str(l.text) }))
        : [],
      no_readings: typeof s.no_readings === "string" ? s.no_readings : null,
      other_rigs: typeof s.other_rigs === "string" ? s.other_rigs : null,
      presence_line: typeof s.presence_line === "string" ? s.presence_line : null,
    },
  };
}

/** **Decode a `body.ground` frame** (every key optional): `null` when it is no grid frame. */
export function parseGroundFrame(raw: unknown): GroundFrame | null {
  const grid = parseMapFrame(raw);
  if (!grid || !isObj(raw)) {
    return null;
  }
  const g = raw.ground;
  if (!isObj(g) || grid.width === 0) {
    return { grid, ground: null };
  }
  const sites: GroundSite[] = Array.isArray(g.sites)
    ? g.sites.filter(isObj).map((s) => ({
        letter: str(s.letter),
        site: str(s.site),
        entry: str(s.entry),
        venue: str(s.venue),
        cell: pair(s.cell) ?? [0, 0],
        drawn: s.drawn === true,
        lat_mdeg: num(s.lat_mdeg),
        lon_mdeg: num(s.lon_mdeg),
        survey: typeof s.survey === "string" ? s.survey : null,
        tenure: str(s.tenure),
        passage: typeof s.passage === "string" ? s.passage : null,
        here: s.here === true,
        best: typeof s.best === "string" ? s.best : null,
        dark: s.dark === true,
        best_line: str(s.best_line),
      }))
    : [];
  const ports: GroundPort[] = Array.isArray(g.ports)
    ? g.ports.filter(isObj).map((p) => ({
        glyph: str(p.glyph),
        port: str(p.port),
        cell: pair(p.cell) ?? [0, 0],
        drawn: p.drawn === true,
        lat_mdeg: num(p.lat_mdeg),
        lon_mdeg: num(p.lon_mdeg),
      }))
    : [];
  return {
    grid,
    ground: {
      body: str(g.body),
      name: str(g.name),
      family: str(g.family),
      family_word: str(g.family_word),
      families: strs(g.families),
      ports,
      sites,
      also: strs(g.also),
      footer: str(g.footer),
      lines: strs(g.lines),
      print: strs(g.print),
    },
  };
}

/** **The act's line for a rig** (lead ruling 6: prefill, never submit): `+tv/<n> <act>`, or
 * `+tv <act>` with no rig. */
export function rigPrefill(slot: number | null, act: string): string {
  return slot === null ? `+tv ${act}` : `+tv/${slot} ${act}`;
}

/** The entry after `current` in `items`, wrapping; the first when `current` is absent. */
function nextIn<T>(items: readonly T[], current: T | null): T | null {
  if (items.length === 0) {
    return null;
  }
  const at = current === null ? -1 : items.indexOf(current);
  return at < 0 ? (items[0] ?? null) : (items[(at + 1) % items.length] ?? null);
}

/** **`t`**: the next rig of the frame's, in slot order, wrapping. */
export function nextRig(slots: readonly number[], current: number | null): number | null {
  return nextIn([...slots].sort((a, b) => a - b), current);
}

/** **`l`**: the family after the one shown, in the frame's `families`, wrapping. */
export function nextFamily(words: readonly string[], shown: string | null): string | null {
  return nextIn(words, shown);
}

/** **`z`**: drive → site → ground → drive. */
export function nextMode(mode: QuarryMode): QuarryMode {
  return mode === "drive" ? "site" : mode === "site" ? "ground" : "drive";
}

/** **`site.map`'s parameters** for a view, a rig and a family (the ground view pulls the site
 * view), as the TUI sends them. */
export function siteParams(
  mode: QuarryMode,
  rig: number | null,
  family: string | null,
): { [key: string]: JsonValue } {
  const params: { [key: string]: JsonValue } = { view: mode === "drive" ? "drive" : "site" };
  if (rig !== null) {
    params.rig = rig;
  }
  if (family !== null) {
    params.family = family;
  }
  return params;
}

/** **`body.ground`'s parameters**: the family; the body defaults on the server. */
export function groundParams(family: string | null): { [key: string]: JsonValue } {
  return family === null ? {} : { family };
}

/** **The site cell a clicked frame cell drives to** (`fx`, `fy` with `fy` from the south edge,
 * as the rows run): `x0 + fx·span + ⌊span/2⌋`, likewise for y — the drive target form `x,y`. */
export function siteCellOf(site: SiteBlock, fx: number, fy: number): [number, number] | null {
  const g = site.grid;
  if (g === null) {
    return null;
  }
  const half = Math.floor(g.span / 2);
  return [g.x0 + fx * g.span + half, g.y0 + fy * g.span + half];
}

/** **A place on the 2:1 body picture**, north at the top: `x = lon/360 × width`, `y = (90 −
 * lat)/180 × height`, from milli-degrees. */
export function groundPoint(
  place: { lat_mdeg: number; lon_mdeg: number },
  width: number,
  height: number,
): { x: number; y: number } {
  return {
    x: (place.lon_mdeg / 360_000) * width,
    y: ((90_000 - place.lat_mdeg) / 180_000) * height,
  };
}

/** **A position along a frozen leg at `t` seconds from its start** — the web's copy of
 * `tacsim::port::site_map::leg_position` (doc 20 §8): the first waypoint before the leg, the
 * last after it, linear between by `at_s`, each step truncated toward zero as the server's
 * integer arithmetic is. */
export function legPosition(points: readonly Waypoint[], t: number): [number, number] | null {
  const first = points[0];
  if (!first) {
    return null;
  }
  if (t <= first.at_s) {
    return [first.x_cm, first.y_cm];
  }
  for (let i = 0; i + 1 < points.length; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (t <= b.at_s) {
      if (b.at_s <= a.at_s) {
        return [b.x_cm, b.y_cm];
      }
      const lerp = (p: number, q: number) => p + Math.trunc(((q - p) * (t - a.at_s)) / (b.at_s - a.at_s));
      return [lerp(a.x_cm, b.x_cm), lerp(a.y_cm, b.y_cm)];
    }
  }
  const last = points[points.length - 1]!;
  return [last.x_cm, last.y_cm];
}

/** **The world's IC instant now, as the client reckons it**: the frame's clock plus the real
 * seconds since it landed times the world's rate; `null` with no clock (nothing moves). */
export function icNow(site: SiteBlock, receivedMs: number, nowMs: number): number | null {
  if (site.clock === null) {
    return null;
  }
  const elapsed = Math.max(0, Math.floor((nowMs - receivedMs) / 1000));
  return site.clock.now_ic + elapsed * site.clock.rate;
}

/** **The site grid's rows, row 0 the south edge, with the viewer's rigs placed at `ic`** (the
 * TUI's `quarry_grid`): the frame's rows; with `ic`, the glyphs under the rigs restored and each
 * rig placed again — along its leg once the leg has begun, else at its frame cell — its slot
 * digit east of it when two or more stand on the ground inside the window (the server's own
 * count: a rig past the window's edge is not on this map), then its `@`. At the frame's own
 * instant these are the frame's rows exactly. */
export function placedRows(frame: SiteFrame, ic: number | null): string[][] {
  const rows = expandRows({ ...frame.grid, units: [] });
  const s = frame.site;
  if (ic === null || s === null || s.grid === null) {
    return rows;
  }
  const { x0, y0, span, cell_cm } = s.grid;
  const put = ([fx, fy]: [number, number], glyph: string) => {
    const row = rows[fy];
    if (row && fx >= 0 && fx < frame.grid.width) {
      row[fx] = glyph;
    }
  };
  for (const [fx, fy, glyph] of s.under) {
    put([fx, fy], glyph);
  }
  const frameCell = ([sx, sy]: [number, number]): [number, number] => [
    Math.floor((sx - x0) / span),
    Math.floor((sy - y0) / span),
  ];
  const inside = ([fx, fy]: [number, number]) =>
    fx >= 0 && fy >= 0 && fx < frame.grid.width && fy < frame.grid.height;
  const placed: [number, [number, number]][] = [];
  for (const r of s.rigs) {
    let at: [number, number] | null = null;
    if (r.leg && ic >= r.leg.started_ic) {
      const p = legPosition(r.leg.waypoints, ic - r.leg.started_ic);
      if (p) {
        at = [Math.floor(p[0] / cell_cm), Math.floor(p[1] / cell_cm)];
      }
    }
    at = at ?? r.cell;
    // Only a rig inside the window counts, as the server's compose counts its own layer of
    // the viewer's rigs: a rig on the ground past the window's edge draws no `@` here and
    // lends no other rig a slot digit.
    if (at && inside(frameCell(at))) {
      placed.push([r.slot, at]);
    }
  }
  if (placed.length >= 2) {
    for (const [slot, [sx, sy]] of placed) {
      const east = frameCell([sx + span, sy]);
      if (inside(frameCell([sx, sy])) && inside(east)) {
        put(east, String(slot));
      }
    }
  }
  for (const [, at] of placed) {
    const cell = frameCell(at);
    if (inside(cell)) {
      put(cell, "@");
    }
  }
  return rows;
}

/** **One colour per glyph** (a glyph's colour, never a glyph): the terrain, the readings, the
 * pits, the units and the marks of the site map, and the ground chart's wash. */
export const GLYPH_COLOURS: Readonly<Record<string, string>> = {
  ",": "#6b5f4f",
  "^": "#4d4f56",
  "=": "#8fb8d1",
  _: "#55524c",
  " ": "#0e1116",
  O: "#2b2a29",
  "1": "#a0c27a",
  "2": "#d5c25c",
  "3": "#e88f4a",
  "+": "#7f8fa6",
  ".": "#3a3d43",
  v: "#9c6b4e",
  V: "#6e4636",
  "#": "#c76d6d",
  H: "#f2c94c",
  o: "#c0c4cc",
  "@": "#ffffff",
  "-": "#5c6a52",
  "*": "#d9a441",
};

/** The colour a glyph is drawn in (a letter or a digit not listed: the mark colour). */
export function glyphColour(glyph: string): string {
  return GLYPH_COLOURS[glyph] ?? "#9fc4d8";
}

/** The ground print's grid rows (after the title and the legend lines), for the flat toggle
 * and the byte-identity pin. */
export function groundPrintRows(ground: GroundBlock, height: number): string[] {
  let skip = 1;
  while (skip < ground.print.length && (ground.print[skip]!.startsWith("legend:") || ground.print[skip]!.startsWith("  "))) {
    skip += 1;
  }
  return ground.print.slice(skip, skip + height);
}
