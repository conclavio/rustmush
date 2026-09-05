// The roll builder (interface plan I6): pickers from the schema's
// categories, the difficulty band, and the `+roll` line.

import { describe, expect, it } from "vitest";

import { buildRollLine, clampDifficulty, traitGroups } from "./rollbuilder";
import { parseSheetSchema } from "./sheet";

const schema = parseSheetSchema({
  ruleset: "toy",
  version: 1,
  sections: [
    {
      id: "attrs",
      label: "Attributes",
      layout: "list",
      fields: [
        { id: "strength", label: "Strength", ty: { kind: "dots", min: 1, max: 5 }, category: "physical", default: 1 },
        { id: "wits", label: "Wits", ty: { kind: "dots", min: 1, max: 5 }, category: "mental", default: 1 },
        { id: "luck", label: "Luck", ty: { kind: "dots", min: 1, max: 5 }, default: 1 },
        { id: "hidden", label: "Hidden", ty: { kind: "dots", min: 1, max: 5 }, category: "physical", staff_only: true, default: 1 },
      ],
    },
    {
      id: "abilities",
      label: "Abilities",
      layout: "list",
      fields: [
        { id: "brawl", label: "Brawl", ty: { kind: "dots", min: 0, max: 5 }, category: "talents", default: 0 },
        { id: "essence", label: "Essence", ty: { kind: "number", min: 0, max: 600 }, category: "special", default: 600 },
      ],
    },
  ],
});

describe("roll builder", () => {
  it("groups dotted, categorised, player-visible traits in schema order", () => {
    const groups = traitGroups(schema);
    expect(groups.map((g) => [g.label, g.traits.map((t) => t.id)])).toEqual([
      ["Physical", ["strength"]],
      ["Mental", ["wits"]],
      ["Talents", ["brawl"]],
    ]);
    expect(traitGroups(null)).toEqual([]);
  });

  it("keeps the difficulty in 3..9", () => {
    expect(clampDifficulty(6)).toBe(6);
    expect(clampDifficulty(1)).toBe(3);
    expect(clampDifficulty(12)).toBe(9);
    expect(clampDifficulty(Number.NaN)).toBe(6);
    expect(clampDifficulty(7.9)).toBe(7);
  });

  it("builds the one line the server already accepts", () => {
    expect(buildRollLine({ first: "strength", second: "brawl", difficulty: 6, spec: false })).toBe(
      "+roll strength+brawl vs 6",
    );
    expect(buildRollLine({ first: "wits", second: null, difficulty: 8, spec: true })).toBe(
      "+roll/spec wits vs 8",
    );
    expect(buildRollLine({ first: " ", second: "brawl", difficulty: 6, spec: false })).toBeNull();
    expect(buildRollLine({ first: "wits", second: "", difficulty: 99, spec: false })).toBe(
      "+roll wits vs 9",
    );
  });
});
