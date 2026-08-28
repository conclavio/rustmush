// SheetStore: keyframe hydration, delta merge, unavailable fallback, clear.

import { describe, expect, it } from "vitest";

import { SheetStore } from "./sheet.svelte";

const keyframe = {
  target: "#7",
  schema: {
    ruleset: "wod",
    version: 1,
    sections: [
      {
        id: "attributes",
        label: "Attributes",
        layout: { grid: { rows: 1, cols: 1 } },
        fields: [{ id: "strength", label: "Strength", ty: { kind: "dots", min: 1, max: 5 }, default: 1 }],
      },
    ],
  },
  approved: { state: "unapproved", approved: false },
  data: { strength: 3 },
};

describe("SheetStore", () => {
  it("starts empty and not loaded", () => {
    const s = new SheetStore();
    expect(s.loaded).toBe(false);
    expect(s.unavailable).toBe(false);
    expect(s.schema).toBeNull();
    expect(s.rulesetId).toBeNull();
  });

  it("applyKeyframe hydrates schema/data/approval/target and clears unavailable", () => {
    const s = new SheetStore();
    s.markUnavailable();
    const ok = s.applyKeyframe(keyframe);
    expect(ok).toBe(true);
    expect(s.loaded).toBe(true);
    expect(s.unavailable).toBe(false);
    expect(s.target).toBe("#7");
    expect(s.rulesetId).toBe("wod");
    expect(s.data.strength).toBe(3);
    expect(s.approval?.approved).toBe(false);
  });

  it("applyKeyframe on a malformed payload leaves state untouched and reports false", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    const ok = s.applyKeyframe({ nonsense: true });
    expect(ok).toBe(false);
    expect(s.data.strength).toBe(3); // unchanged
  });

  it("applyDelta shallow-merges data and refreshes approval", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    s.applyDelta({ data: { strength: 4 }, approved: { state: "approved", approved: true } });
    expect(s.data.strength).toBe(4);
    expect(s.approval?.approved).toBe(true);
    // Untouched fields survive the merge.
    expect(s.target).toBe("#7");
  });

  it("applyDelta on garbage is a no-op", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    s.applyDelta(null);
    s.applyDelta("nope");
    expect(s.data.strength).toBe(3);
  });

  it("markUnavailable flips the graceful-degradation flag", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    s.markUnavailable();
    expect(s.unavailable).toBe(true);
    expect(s.loaded).toBe(false);
  });

  it("clear resets everything", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    s.clear();
    expect(s.schema).toBeNull();
    expect(s.data).toEqual({});
    expect(s.approval).toBeNull();
    expect(s.target).toBeNull();
    expect(s.loaded).toBe(false);
    expect(s.unavailable).toBe(false);
  });
});
