// The sky chart on the web (doc 19 cut A.3b; doc 19 §5): the wire type of the
// `sky.chart` panel, its decoder, and the pure helpers `ChartScreen.svelte`
// paints from — the radial scale the server's own raster uses, the points a
// body projects to, the route polylines and the table lines.
//
// Everything here is pure and DOM-free, so vitest pins what the player sees
// without a browser. The frame is a GRID frame (`schema.shape == "grid"`, the
// shape `atlas.map` and the arena map already use) with a `chart` block beside
// the grid: the grid is the server's flat render — the same character map the
// TUI shows in its chart pane and `+chart/map` prints in line mode — and the
// `chart` block is the vector data this screen draws its own picture from, so a
// client that cannot draw still has the text and a client that can draws the
// same sky.
//
// The payload is a `serde_json::Value` on the server — outside ts-rs — so these
// types are hand-written, and the decoder is tolerant of a missing key and of
// an unknown one: the frame evolves additively and neither an older server nor
// a newer one may make this client throw.

import { parseMapFrame } from "./stores/map.svelte";
import type { AtlasMapFrame } from "./proto";

/** The sky chart's feed (`tacsim::port::chart_panel::SKY_CHART_FEED`). */
export const SKY_CHART_FEED = "sky.chart";

/** One body of the chart. */
export interface ChartBody {
  id: string;
  label: string;
  /** The glyph the raster draws it with (a moon shares its parent's cell). */
  glyph: string;
  /** The body this one orbits, for a moon; `null` about the Sun. */
  parent: string | null;
  /** Heliocentric distance, AU. */
  r_au: number;
  /** Ecliptic longitude and latitude, degrees. */
  lon_deg: number;
  lat_deg: number;
  /** The heliocentric position, AU, ecliptic J2000. */
  x_au: number;
  y_au: number;
  z_au: number;
  /** The orbital period about the Sun (a moon's is its primary's), days. */
  period_days: number;
  /** Whether the chart is read from this body. */
  here: boolean;
  /** The distance from here, AU, and the one-way light lag, seconds; `null`
   * for the body the chart is read from. */
  from_here_au: number | null;
  lag_s: number | null;
  /** The raster cell the body is drawn in. */
  cell: [number, number];
}

/** One leg of a route, body to body. */
export interface ChartLeg {
  from: string;
  to: string;
}

/** One planned passage. */
export interface ChartPassage {
  total_s: number;
  burn_t: number;
  crew_days: number;
}

/** The direct chord's figures. */
export interface ChartDirect {
  km: number;
  total_s: number;
  burn_t: number;
}

/** One destination the vantage port clears for. */
export interface ChartRoute {
  to: string;
  venue: string;
  /** The destination's body. */
  body: string;
  legs: ChartLeg[];
  /** Planned now at the vantage's load, and empty; `null` when not plannable. */
  loaded: ChartPassage | null;
  empty: ChartPassage | null;
  /** The direct chord, priced beside the listed route. */
  direct: ChartDirect | null;
}

/** One day of a destination's window scan. */
export interface ChartDay {
  offset_days: number;
  total_s: number;
  burn_t: number;
}

/** A destination's window over the next IC year. */
export interface ChartWindow {
  to: string;
  now: ChartPassage;
  fastest: ChartDay | null;
  cheapest: ChartDay | null;
  worst: ChartDay | null;
}

/** The vantage the chart is read from. */
export interface ChartVantage {
  port: string;
  body: string;
  hull: string;
  payload_kg: number;
  occupants: number;
}

/** The chart's vector block. */
export interface ChartData {
  date: string;
  ic_now: number;
  model: string;
  rate_permille: number;
  vantage: ChartVantage;
  /** The radial scale of the raster: `kind` is `log`, `au_max` the AU at the
   * edge, `r0_au` the knee. */
  scale: { kind: string; au_max: number; r0_au: number };
  bodies: ChartBody[];
  routes: ChartRoute[];
  window: ChartWindow[];
}

/** A `sky.chart` keyframe: the grid half and the chart half. */
export interface ChartFrame {
  grid: AtlasMapFrame;
  chart: ChartData;
}

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function num(v: unknown, fallback = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}

function optNum(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function str(v: unknown, fallback = ""): string {
  return typeof v === "string" ? v : fallback;
}

function cell(v: unknown): [number, number] {
  return Array.isArray(v) && v.length === 2 ? [num(v[0]), num(v[1])] : [0, 0];
}

function passage(v: unknown): ChartPassage | null {
  if (!isObj(v)) {
    return null;
  }
  return { total_s: num(v.total_s), burn_t: num(v.burn_t), crew_days: num(v.crew_days) };
}

function day(v: unknown): ChartDay | null {
  if (!isObj(v)) {
    return null;
  }
  return { offset_days: num(v.offset_days), total_s: num(v.total_s), burn_t: num(v.burn_t) };
}

function body(v: unknown): ChartBody | null {
  if (!isObj(v) || typeof v.id !== "string") {
    return null;
  }
  return {
    id: v.id,
    label: str(v.label, v.id),
    glyph: str(v.glyph, "?").slice(0, 1) || "?",
    parent: typeof v.parent === "string" ? v.parent : null,
    r_au: num(v.r_au),
    lon_deg: num(v.lon_deg),
    lat_deg: num(v.lat_deg),
    x_au: num(v.x_au),
    y_au: num(v.y_au),
    z_au: num(v.z_au),
    period_days: num(v.period_days),
    here: v.here === true,
    from_here_au: optNum(v.from_here_au),
    lag_s: optNum(v.lag_s),
    cell: cell(v.cell),
  };
}

function route(v: unknown): ChartRoute | null {
  if (!isObj(v) || typeof v.to !== "string") {
    return null;
  }
  const legs: ChartLeg[] = Array.isArray(v.legs)
    ? v.legs
        .map((l): ChartLeg | null =>
          isObj(l) && typeof l.from === "string" && typeof l.to === "string" ? { from: l.from, to: l.to } : null,
        )
        .filter((l): l is ChartLeg => l !== null)
    : [];
  const direct = isObj(v.direct)
    ? { km: num(v.direct.km), total_s: num(v.direct.total_s), burn_t: num(v.direct.burn_t) }
    : null;
  return {
    to: v.to,
    venue: str(v.venue),
    body: str(v.body),
    legs,
    loaded: passage(v.loaded),
    empty: passage(v.empty),
    direct,
  };
}

function window(v: unknown): ChartWindow | null {
  if (!isObj(v) || typeof v.to !== "string") {
    return null;
  }
  const now = passage(v.now);
  if (!now) {
    return null;
  }
  return { to: v.to, now, fastest: day(v.fastest), cheapest: day(v.cheapest), worst: day(v.worst) };
}

/** Decode a `sky.chart` keyframe; `null` when it is not one. */
export function parseChartFrame(raw: unknown): ChartFrame | null {
  const grid = parseMapFrame(raw);
  if (!grid || !isObj(raw) || !isObj(raw.chart)) {
    return null;
  }
  const c = raw.chart;
  const vantage: ChartVantage = isObj(c.vantage)
    ? {
        port: str(c.vantage.port),
        body: str(c.vantage.body),
        hull: str(c.vantage.hull),
        payload_kg: num(c.vantage.payload_kg),
        occupants: num(c.vantage.occupants, 1),
      }
    : { port: "", body: "", hull: "", payload_kg: 0, occupants: 1 };
  const scale = isObj(c.scale)
    ? {
        kind: str(c.scale.kind, "log"),
        au_max: num(c.scale.au_max, 10) || 10,
        r0_au: num(c.scale.r0_au, R0_AU) || R0_AU,
      }
    : { kind: "log", au_max: 10, r0_au: R0_AU };
  const bodies = Array.isArray(c.bodies) ? c.bodies.map(body).filter((b): b is ChartBody => b !== null) : [];
  const routes = Array.isArray(c.routes) ? c.routes.map(route).filter((r): r is ChartRoute => r !== null) : [];
  const windows = Array.isArray(c.window)
    ? c.window.map(window).filter((w): w is ChartWindow => w !== null)
    : [];
  return {
    grid,
    chart: {
      date: str(c.date),
      ic_now: num(c.ic_now),
      model: str(c.model),
      rate_permille: num(c.rate_permille),
      vantage,
      scale,
      bodies,
      routes,
      window: windows,
    },
  };
}

/** The radial scale's knee, AU (`tacsim::port::chart_panel::R0_AU`). */
export const R0_AU = 0.3;

/**
 * **The radial scale** (doc 19 §5, cut A.3b): the server's raster and this
 * screen's picture both place a body at `ln(1 + r / r0) / ln(1 + au_max / r0)`
 * of the radius — logarithmic past the knee — so Mercury at 0.39 AU and Saturn
 * at 9.5 AU share one picture with Earth, Mars and the Belt still apart; a
 * linear scale would crush the inner system into the Sun's own cell and a
 * square root packed six orbits into seven rows. The value is the fraction of
 * the picture's radius, `0..=1` (clamped: a body past the edge sits on it).
 */
export function radialFraction(rAu: number, auMax: number, r0 = R0_AU): number {
  if (!(auMax > 0) || !(rAu > 0) || !(r0 > 0)) {
    return 0;
  }
  return Math.min(1, Math.log(1 + rAu / r0) / Math.log(1 + auMax / r0));
}

/** A point of the picture: `cx`, `cy` in the picture's own units. */
export interface ChartPoint {
  id: string;
  x: number;
  y: number;
}

/**
 * Where each body is drawn in a square picture `size` across, the Sun at the
 * centre, ecliptic north up (y grows downward on a screen, so the ecliptic
 * y is negated) and the longitude measured from +x anticlockwise as the sky
 * does. The radius used is `radialFraction` of the body's heliocentric
 * distance, the direction its longitude: a moon, at this scale, lands on its
 * parent.
 */
export function bodyPoints(chart: ChartData, size: number): ChartPoint[] {
  const half = size / 2;
  const radius = half * 0.94;
  return chart.bodies.map((b) => {
    const f = radialFraction(b.r_au, chart.scale.au_max, chart.scale.r0_au);
    const theta = (b.lon_deg * Math.PI) / 180;
    return { id: b.id, x: half + radius * f * Math.cos(theta), y: half - radius * f * Math.sin(theta) };
  });
}

/** The ring a body's orbit draws, as the picture's radius for its distance. */
export function orbitRadius(b: ChartBody, chart: ChartData, size: number): number {
  return (size / 2) * 0.94 * radialFraction(b.r_au, chart.scale.au_max, chart.scale.r0_au);
}

/** One light-minute, AU (`mush_pixel::chart::AU_PER_LIGHT_MINUTE`). */
export const AU_PER_LIGHT_MINUTE = 0.1202393;

/** The light-lag rings about the viewer's body, light-minutes (doc 21 cut
 * G.5c, the owner's default): doubling, which spaces evenly on the log
 * scale, and the same four the terminal's picture draws. */
export const LAG_RING_MINUTES = [3, 6, 12, 24] as const;

/** A light-lag ring: the points `minutes` of light from the viewer's body
 * (the chart's `here`), as picture points in a square `size` across — a
 * circle about the body in space, sampled through the log scale, which bends
 * it as it bends distance. Empty on a chart with no `here`. */
export function lagRing(chart: ChartData, minutes: number, size: number, steps = 180): ChartPoint[] {
  const here = chart.bodies.find((b) => b.here);
  if (!here) return [];
  const lon = (here.lon_deg * Math.PI) / 180;
  const [hx, hy] = [here.r_au * Math.cos(lon), here.r_au * Math.sin(lon)];
  const radius = minutes * AU_PER_LIGHT_MINUTE;
  const half = size / 2;
  const full = half * 0.94;
  const out: ChartPoint[] = [];
  for (let i = 0; i <= steps; i += 1) {
    const a = (2 * Math.PI * i) / steps;
    const [wx, wy] = [hx + radius * Math.cos(a), hy + radius * Math.sin(a)];
    const f = radialFraction(Math.hypot(wx, wy), chart.scale.au_max, chart.scale.r0_au);
    const theta = Math.atan2(wy, wx);
    out.push({ id: `${minutes}`, x: half + full * f * Math.cos(theta), y: half - full * f * Math.sin(theta) });
  }
  return out;
}

/** The picture's caption: the scale as drawn (logarithmic past the knee)
 * and what the rings are. */
export function scaleCaption(chart: ChartData): string {
  return `scale: logarithmic past ${chart.scale.r0_au} AU, the edge at ${chart.scale.au_max.toFixed(1)} AU; rings: light-minutes from here`;
}

/** A route's polyline through its legs' bodies, as picture points; a leg
 * whose body is not on the chart is skipped. */
export function routePolyline(route: ChartRoute, points: readonly ChartPoint[]): ChartPoint[] {
  const by = new Map(points.map((p) => [p.id, p]));
  const out: ChartPoint[] = [];
  for (const [i, leg] of route.legs.entries()) {
    const from = by.get(leg.from);
    const to = by.get(leg.to);
    if (i === 0 && from) {
      out.push(from);
    }
    if (to) {
      out.push(to);
    }
  }
  return out;
}

/** Seconds as `h.h h` (the TUI prints the same figure from the server). */
export function hoursText(secs: number): string {
  return `${(secs / 3600).toFixed(1)} h`;
}

/** Seconds of light lag as `Nm SSs light`. */
export function lagText(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${m}m ${String(s).padStart(2, "0")}s light`;
}

/** The sky table's lines, one per body, nearest the Sun first — what the
 * screen prints beside the picture and what `+chart` prints in line mode. */
export function bodyLines(chart: ChartData): string[] {
  const sorted = [...chart.bodies].sort((a, b) => a.r_au - b.r_au);
  return sorted.map((b) => {
    const here =
      b.here || b.from_here_au === null
        ? "here"
        : `${b.from_here_au.toFixed(3)} AU from here${b.lag_s === null ? "" : `, ${lagText(b.lag_s)}`}`;
    const moon = b.parent ? `  moon of ${b.parent}` : "";
    return `${b.id.padEnd(9)} r ${b.r_au.toFixed(3).padStart(6)} AU  lon ${b.lon_deg.toFixed(1).padStart(6)}  lat ${(b.lat_deg >= 0 ? "+" : "") + b.lat_deg.toFixed(1)}  ${here}${moon}`;
  });
}

/** One line per destination: the listed route now, empty, and the direct chord. */
export function routeLines(chart: ChartData): string[] {
  return chart.routes.map((r) => {
    const parts: string[] = [];
    if (r.loaded) {
      parts.push(
        `${hoursText(r.loaded.total_s)} at this load (${Math.floor(chart.vantage.payload_kg / 1000)} t aboard ${chart.vantage.hull}: burn ${r.loaded.burn_t} t, ${r.loaded.crew_days} crew-days)`,
      );
    } else {
      parts.push("not plannable at this load");
    }
    if (r.empty) {
      parts.push(`empty ${hoursText(r.empty.total_s)} and ${r.empty.burn_t} t`);
    }
    if (r.direct) {
      parts.push(
        `the direct chord ${(r.direct.km / 149_597_871).toFixed(3)} AU would be ${hoursText(r.direct.total_s)} and ${r.direct.burn_t} t`,
      );
    }
    return `${r.to}: ${parts.join("; ")}.`;
  });
}

/** One line per destination's window: the fastest, cheapest and worst day of
 * the next IC year against now; "every day is today" under a constant sky. */
export function windowLines(chart: ChartData): string[] {
  return chart.window.map((w) => {
    if (!w.fastest) {
      return `${w.to}: now ${hoursText(w.now.total_s)}, burn ${w.now.burn_t} t; the sky is constant — every day is today.`;
    }
    const describe = (label: string, d: ChartDay | null): string | null => {
      if (!d) {
        return null;
      }
      const dh = d.total_s - w.now.total_s;
      const dt = d.burn_t - w.now.burn_t;
      return `${label} in ${d.offset_days} IC days: ${hoursText(d.total_s)}, burn ${d.burn_t} t (${dh < 0 ? "-" : "+"}${hoursText(Math.abs(dh))} and ${dt < 0 ? "-" : "+"}${Math.abs(dt)} t)`;
    };
    const parts = [
      `now ${hoursText(w.now.total_s)}, burn ${w.now.burn_t} t`,
      describe("fastest", w.fastest),
      w.cheapest && w.fastest && w.cheapest.offset_days !== w.fastest.offset_days
        ? describe("cheapest", w.cheapest)
        : null,
      describe("worst", w.worst),
    ].filter((p): p is string => p !== null);
    return `${w.to}: ${parts.join("; ")}.`;
  });
}

/** The heading the screen prints. */
export function chartHeading(chart: ChartData): string {
  const motion = chart.rate_permille === 0 ? "constant" : `moving, rate ${chart.rate_permille}`;
  return `The sky on ${chart.date} (${chart.model}; ${motion}), from ${chart.vantage.port} on ${chart.vantage.body}`;
}
