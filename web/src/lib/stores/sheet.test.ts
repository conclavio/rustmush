// SheetStore: keyframe hydration, the I13 delta revision watermark + self-heal,
// unavailable fallback, clear.

import { describe, expect, it } from "vitest";

import { SheetStore } from "./sheet.svelte";

const keyframe = {
  target: "#7",
  revision: 5,
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

  it("applyKeyframe seeds the revision watermark", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe);
    expect(s.revision).toBe(5);
  });

  it("applyDelta applies on a matching base_revision: replaces data, advances revision", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe); // revision 5
    const r = s.applyDelta({
      revision: 6,
      base_revision: 5,
      data: { strength: 4 },
      approved: { state: "approved", approved: true },
    });
    expect(r).toBe("applied");
    expect(s.data.strength).toBe(4);
    expect(s.approval?.approved).toBe(true);
    expect(s.revision).toBe(6);
  });

  it("applyDelta reports 'resync' on a base gap and leaves data + revision intact", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe); // revision 5
    const r = s.applyDelta({ revision: 8, base_revision: 7, data: { strength: 9 } });
    expect(r).toBe("resync");
    expect(s.data.strength).toBe(3);
    expect(s.revision).toBe(5);
  });

  it("applyDelta reports 'stale' on an old or duplicate revision", () => {
    const s = new SheetStore();
    s.applyKeyframe(keyframe); // revision 5
    expect(s.applyDelta({ revision: 5, base_revision: 4, data: { strength: 4 } })).toBe("stale");
    expect(s.applyDelta({ revision: 3, base_revision: 2, data: { strength: 4 } })).toBe("stale");
    expect(s.data.strength).toBe(3);
  });

  it("applyDelta before any keyframe is 'stale'; a malformed frame is 'resync'", () => {
    const s = new SheetStore();
    // Not loaded yet — the pending hydration keyframe will seed it.
    expect(s.applyDelta({ revision: 1, base_revision: 0, data: {} })).toBe("stale");
    s.applyKeyframe(keyframe);
    expect(s.applyDelta(null)).toBe("resync");
    expect(s.applyDelta("nope")).toBe("resync");
    expect(s.applyDelta({ data: { strength: 4 } })).toBe("resync"); // no revision/base
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
    expect(s.revision).toBe(0);
    expect(s.unavailable).toBe(false);
  });
});
