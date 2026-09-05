// The roll builder (interface plan I6): pickers from the sheet schema's
// `category` tags — any dotted field with a category is a trait — a
// difficulty in 3..9, the specialty switch, and the one line the server's
// `+roll` command already accepts. (The `/wp` switch is not offered: the
// server routes that roll through the authoritative action surface and
// refuses the switch.) No trait or category name lives here; the schema is
// the source (D-I-2).

import type { SheetSchema } from "./sheet";

export interface TraitOption {
  id: string;
  label: string;
}

export interface TraitGroup {
  /** The schema's category id. */
  category: string;
  /** The category, capitalised for a picker heading. */
  label: string;
  traits: TraitOption[];
}

export const DIFFICULTY_MIN = 3;
export const DIFFICULTY_MAX = 9;
export const DIFFICULTY_DEFAULT = 6;

/** Every dotted, categorised field, grouped by category in schema order. */
export function traitGroups(schema: SheetSchema | null): TraitGroup[] {
  if (!schema) {
    return [];
  }
  const groups: TraitGroup[] = [];
  for (const section of schema.sections) {
    for (const f of section.fields) {
      if (f.ty.kind !== "dots" || !f.category || f.staff_only) {
        continue;
      }
      let g = groups.find((x) => x.category === f.category);
      if (!g) {
        g = { category: f.category, label: capitalize(f.category), traits: [] };
        groups.push(g);
      }
      g.traits.push({ id: f.id, label: f.label });
    }
  }
  return groups;
}

/** A difficulty inside the classic 3..9 band; a bad number is the default. */
export function clampDifficulty(d: number): number {
  if (!Number.isFinite(d)) {
    return DIFFICULTY_DEFAULT;
  }
  return Math.max(DIFFICULTY_MIN, Math.min(DIFFICULTY_MAX, Math.trunc(d)));
}

export interface RollSpec {
  first: string;
  second: string | null;
  difficulty: number;
  spec: boolean;
}

/**
 * The `+roll` line for a spec: `+roll[/spec] <a>[+<b>] vs <d>`, or `null`
 * without a first trait. Trait ids are passed through untouched — the
 * server resolves them against the sheet.
 */
export function buildRollLine(spec: RollSpec): string | null {
  const first = spec.first.trim();
  if (first.length === 0) {
    return null;
  }
  const second = spec.second?.trim() ?? "";
  const verb = `+roll${spec.spec ? "/spec" : ""}`;
  const pool = second.length > 0 ? `${first}+${second}` : first;
  return `${verb} ${pool} vs ${clampDifficulty(spec.difficulty)}`;
}

function capitalize(s: string): string {
  return s.length > 0 ? s.charAt(0).toUpperCase() + s.slice(1) : s;
}
