// Shared widgets (interface plan I6): clamping, grouping, the condition
// map and table cells — none may throw on a bad input.

import { describe, expect, it } from "vitest";

import {
  CONDITION_GLYPHS,
  conditionGlyph,
  gaugeFill,
  gaugeText,
  groupBoxes,
  tableCells,
} from "./widgets";

describe("widgets", () => {
  it("the gauge clamps and rights swapped bounds", () => {
    expect(gaugeFill(6, 0, 10, 8)).toBe(5);
    expect(gaugeFill(40, 0, 10, 8)).toBe(8);
    expect(gaugeFill(-3, 0, 10, 8)).toBe(0);
    expect(gaugeFill(5, 10, 0, 8)).toBe(4);
    expect(gaugeFill(5, 5, 5, 8)).toBe(0);
    expect(gaugeFill(5, 0, 10, 0)).toBe(0);
    expect(gaugeText(6, 0, 10)).toBe("▰▰▰▰▰▱▱▱ 6/10");
  });

  it("boxes group by three and survive an empty or odd track", () => {
    expect(groupBoxes([1, 2, 3, 4, 5, 6, 7])).toEqual([[1, 2, 3], [4, 5, 6], [7]]);
    expect(groupBoxes([])).toEqual([]);
    expect(groupBoxes([1, 2], 0)).toEqual([[1], [2]]);
    expect(groupBoxes([1, 2], -2)).toEqual([[1], [2]]);
  });

  it("the condition map is opaque ids through data", () => {
    expect(conditionGlyph("ok")).toBe("✓");
    expect(conditionGlyph("warn")).toBe("◐");
    expect(conditionGlyph("down")).toBe("✗");
    expect(conditionGlyph("mystery")).toBe("·");
    expect(conditionGlyph("toString")).toBe("·");
    expect(conditionGlyph("jammed", { ...CONDITION_GLYPHS, jammed: "J" })).toBe("J");
  });

  it("table cells follow the declared columns", () => {
    const cols = [
      { key: "a", label: "A" },
      { key: "b", label: "B", align: "right" as const },
    ];
    expect(tableCells(cols, { a: "x", b: 3 })).toEqual(["x", "3"]);
    expect(tableCells(cols, { b: null })).toEqual(["", ""]);
  });
});
