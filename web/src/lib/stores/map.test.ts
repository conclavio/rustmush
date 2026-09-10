// MapStore: a keyframe replaces the grid, a delta moves the units and keeps
// its note, another window's delta is ignored, and the expanded rows agree
// with the renderer's rules (clip long runs, pad short rows, overlay units).

import { describe, expect, it } from "vitest";

import { expandRows, MapStore, parseMapFrame } from "./map.svelte";

const frame = () => ({
  schema: { id: "atlas.map", version: 1, shape: "grid" },
  window: 3,
  revision: 2,
  tick: 0,
  title: "Coffee Shop",
  surface: "floor 0",
  floors: ["ground", "floor 0"],
  origin: [31200, 24000],
  cell_cm: 100,
  width: 5,
  height: 3,
  rows: [[["#", 9]], [["#", 1], ["_", 3], ["#", 1]], [["#", 2], ["+", 1]]],
  legend: [["#", "wall"], ["_", "floor"]],
  units: [
    { id: "#5", name: "Rae", cell: [1, 1], glyph: "@", you: true },
    { id: "#6", name: "Kit", cell: [3, 1], glyph: "o", you: false },
    { id: "bad", name: "x", cell: [1] },
  ],
  you: [1, 1],
});

describe("MapStore", () => {
  it("parses a keyframe and expands its rows like the text renderer", () => {
    const parsed = parseMapFrame(frame());
    expect(parsed).not.toBeNull();
    expect(parsed?.units.map((u) => u.id)).toEqual(["#5", "#6"]);
    expect(expandRows(parsed!)).toEqual([
      ["#", "#", "#", "#", "#"],
      ["#", "@", "_", "o", "#"],
      ["#", "#", "+", " ", " "],
    ]);
  });

  it("refuses a frame that is not a grid", () => {
    expect(parseMapFrame({ schema: { shape: "list" }, window: 1 })).toBeNull();
    expect(parseMapFrame("no")).toBeNull();
    const store = new MapStore();
    expect(store.applyKeyframe({ schema: { shape: "list" } })).toBe(false);
    expect(store.loaded).toBe(false);
  });

  it("moves the units on a delta and keeps the note; another window is ignored", () => {
    const store = new MapStore();
    expect(store.applyKeyframe(frame())).toBe(true);
    store.applyDelta({
      schema: { shape: "grid" },
      window: 3,
      revision: 2,
      units: [{ id: "#5", name: "Rae", cell: [2, 1], glyph: "@", you: true }],
      you: [2, 1],
      note: "Rae moves east.",
    });
    expect(store.frame?.units).toHaveLength(1);
    expect(store.frame?.you).toEqual([2, 1]);
    expect(store.notes).toEqual(["Rae moves east."]);
    store.applyDelta({ schema: { shape: "grid" }, window: 9, units: [], note: "elsewhere" });
    expect(store.frame?.units).toHaveLength(1);
    expect(store.notes).toEqual(["Rae moves east."]);
    store.clear();
    expect(store.frame).toBeNull();
  });
});
