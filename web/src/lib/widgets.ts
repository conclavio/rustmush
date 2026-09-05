// Shared widgets (interface plan I6): the pure halves of the gauge, the
// grouped track, the key/value list, the table and the condition map. They
// know nothing about any ruleset — a widget takes numbers, boxes and opaque
// ids and clamps whatever it is given; the `.vitals` frame is the only
// contract (D-I-2). The TUI carries the same rules in `tui/render.rs`.

/** The gauge glyphs (`▰▱`), digits alongside. */
export const GAUGE_ON = "▰";
export const GAUGE_OFF = "▱";
export const GAUGE_CELLS = 8;

/**
 * How many of `cells` a bounded value fills, clamped into `[min, max]`;
 * swapped bounds are righted and an empty span fills nothing.
 */
export function gaugeFill(current: number, min: number, max: number, cells = GAUGE_CELLS): number {
  const lo = Math.min(min, max);
  const hi = Math.max(min, max);
  const span = hi - lo;
  const n = Math.max(0, Math.trunc(cells));
  if (span <= 0 || n === 0) {
    return 0;
  }
  const v = Math.max(lo, Math.min(hi, current)) - lo;
  return Math.round((v / span) * n);
}

/** The gauge as text: `▰▰▰▱▱▱▱▱ 6/10`. */
export function gaugeText(current: number, min: number, max: number, cells = GAUGE_CELLS): string {
  const filled = gaugeFill(current, min, max, cells);
  const n = Math.max(0, Math.trunc(cells));
  const bar = GAUGE_ON.repeat(filled) + GAUGE_OFF.repeat(Math.max(0, n - filled));
  return `${bar} ${current}/${Math.max(min, max)}`;
}

/**
 * A track's boxes in groups of `per` (a monitor's rows of three); an
 * empty track is no groups, a bad `per` is one.
 */
export function groupBoxes<T>(boxes: readonly T[], per = 3): T[][] {
  const size = Number.isInteger(per) && per > 0 ? per : 1;
  const out: T[][] = [];
  for (let i = 0; i < boxes.length; i += size) {
    out.push(boxes.slice(i, i + size));
  }
  return out;
}

/** The condition glyphs a state id maps to: fine, impaired, out. */
export const CONDITION_GLYPHS: Readonly<Record<string, string>> = {
  ok: "✓",
  warn: "◐",
  crit: "✗",
  down: "✗",
};

/**
 * The condition map: an opaque id through a display map (the default is
 * the vitals states; a ruleset may ship its own as data), `·` when the
 * map has no entry so an unknown id never throws.
 */
export function conditionGlyph(id: string, map: Readonly<Record<string, string>> = CONDITION_GLYPHS): string {
  return Object.prototype.hasOwnProperty.call(map, id) ? map[id]! : "·";
}

/** A table column as the pane spec declares it. */
export interface TableColumn {
  key: string;
  label: string;
  align?: "left" | "right";
}

/** Cells as text, in column order; a missing value is an empty cell. */
export function tableCells(columns: readonly TableColumn[], row: Readonly<Record<string, unknown>>): string[] {
  return columns.map((c) => {
    const v = row[c.key];
    if (v === null || v === undefined) {
      return "";
    }
    return typeof v === "string" ? v : String(v);
  });
}
