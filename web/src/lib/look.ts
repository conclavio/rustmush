// A unit's look in the web client (doc 22 cut U.4): the doll its damage is
// painted onto, read from the SAME look files the gateway's renderers read —
// `crates/mush-pixel/looks/*.json`, bundled here at build time — so the
// cockpit tab's doll and the terminal's are one grid. The shipped files are
// checked where they live (`mush_pixel::look::Look::parse` and the
// `unit_looks` contract test), so this side reads them leniently: a malformed
// look is no look, and a unit with no look gets its section rows alone.
//
// Pure, pinned by vitest: what a part shows (`dollCondition`), the tone that
// tells two touching parts apart (`dollTones`, the same greedy pick as
// `Doll::tones`), and the colour a cell is drawn in (`dollColour`).

import type { CockpitSection } from "./cockpit";

/** What one doll character stands for. */
export interface DollPart {
  section: string;
  facet: string | null;
}

/** A doll: a grid of cells, each empty or a part. */
export interface Doll {
  /** `top` (a vehicle from above, front up) or `upright` (a walker). */
  view: "top" | "upright";
  width: number;
  height: number;
  parts: DollPart[];
  /** Row by row from the top: the part's index, or `null`. */
  cells: (number | null)[];
}

/** One unit type's look, as far as the web reads it. */
export interface Look {
  units: string[];
  label: string;
  short: string;
  doll: Doll | null;
}

/** The look files, bundled. */
const FILES = import.meta.glob("../../../crates/mush-pixel/looks/*.json", {
  eager: true,
  import: "default",
}) as Record<string, unknown>;

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** Read one doll, or `null` if it does not hold together. The legend's
 * characters are taken in sorted order, as the Rust reader (a sorted map)
 * takes them, so a part's index — and its tone — is the same on both sides. */
export function parseDoll(raw: unknown): Doll | null {
  if (!isObj(raw) || !Array.isArray(raw.rows) || !isObj(raw.legend)) return null;
  const view = raw.view === "upright" ? "upright" : raw.view === "top" ? "top" : null;
  const rows = raw.rows.filter((r): r is string => typeof r === "string");
  if (view === null || rows.length === 0 || rows.length !== raw.rows.length) return null;
  const width = [...(rows[0] ?? "")].length;
  if (width === 0 || rows.some((r) => [...r].length !== width)) return null;
  const legend = raw.legend;
  const keys = Object.keys(legend).sort();
  const parts: DollPart[] = [];
  for (const key of keys) {
    const p = legend[key];
    if (!isObj(p) || typeof p.section !== "string") return null;
    parts.push({ section: p.section, facet: typeof p.facet === "string" ? p.facet : null });
  }
  const cells: (number | null)[] = [];
  for (const row of rows) {
    for (const c of row) {
      if (c === "." || c === " ") {
        cells.push(null);
        continue;
      }
      const i = keys.indexOf(c);
      if (i < 0) return null;
      cells.push(i);
    }
  }
  return { view, width, height: rows.length, parts, cells };
}

/** Read one look, or `null`. */
export function parseLook(raw: unknown): Look | null {
  if (!isObj(raw) || !Array.isArray(raw.units)) return null;
  const units = raw.units.filter((u): u is string => typeof u === "string");
  if (units.length === 0 || typeof raw.label !== "string" || typeof raw.short !== "string") {
    return null;
  }
  return {
    units,
    label: raw.label,
    short: raw.short,
    doll: raw.doll === undefined ? null : parseDoll(raw.doll),
  };
}

/** Every shipped look that reads. */
export const LOOKS: Look[] = Object.values(FILES).flatMap((raw) => {
  const look = parseLook(raw);
  return look ? [look] : [];
});

/** The look of unit type `unitType`, or `null`. */
export function lookFor(unitType: string | null): Look | null {
  if (unitType === null) return null;
  return LOOKS.find((l) => l.units.includes(unitType)) ?? null;
}

/** What is left of each part, `[left, max]`: a named facet's armour where the
 * section has it, else the section's internals (none once destroyed); `null`
 * for a part whose section the frame lacks. */
export function dollCondition(doll: Doll, sections: CockpitSection[]): ([number, number] | null)[] {
  return doll.parts.map((part) => {
    const sec = sections.find((s) => s.id === part.section);
    if (!sec) return null;
    const facet = part.facet === null ? undefined : sec.facets.find((f) => f.id === part.facet);
    if (facet) return [facet.armor, facet.armor_max];
    return [sec.condition === "destroyed" ? 0 : sec.internals, sec.internals_max];
  });
}

/** A tone a part, `0` or `1`, so touching parts in the same state are told
 * apart: greedily in part order, each the tone none of its already-toned
 * neighbours has, or `0` when both are taken (`Doll::tones`). */
export function dollTones(doll: Doll): number[] {
  const n = doll.parts.length;
  const touches: number[][] = Array.from({ length: n }, () => []);
  const at = (x: number, y: number): number | null =>
    x < doll.width && y < doll.height ? (doll.cells[y * doll.width + x] ?? null) : null;
  for (let y = 0; y < doll.height; y += 1) {
    for (let x = 0; x < doll.width; x += 1) {
      const a = at(x, y);
      if (a === null) continue;
      for (const b of [at(x + 1, y), at(x, y + 1)]) {
        if (b !== null && b !== a) {
          touches[a]?.push(b);
          touches[b]?.push(a);
        }
      }
    }
  }
  const tone: (number | null)[] = Array.from({ length: n }, () => null);
  for (let part = 0; part < n; part += 1) {
    const taken = (touches[part] ?? []).flatMap((b) => {
      const t = tone[b];
      return t === null || t === undefined ? [] : [t];
    });
    tone[part] = taken.includes(0) && !taken.includes(1) ? 1 : 0;
  }
  return tone.map((t) => t ?? 0);
}

/** The colour a doll cell is drawn in: green, amber, red, or grey when gone
 * (the instruments' ramp), a lighter shade at tone `1`. */
export function dollColour(left: number, max: number, tone: number): string {
  const light = tone === 1;
  if (max <= 0 || left <= 0) return light ? "#6a6a72" : "#4a4a52";
  if (left * 3 >= max * 2) return light ? "#7ad88a" : "#46be5a";
  if (left * 3 >= max) return light ? "#f5d26e" : "#e6b432";
  return light ? "#f0786a" : "#dc3c32";
}
