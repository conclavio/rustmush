// VitalsStore: frames replace, a lower revision is ignored, dominant first.

import { describe, expect, it } from "vitest";

import { boxGlyph, orderedVitals, parseVitalsFrame, vitalPct, VitalsStore } from "./vitals.svelte";

const frame = (revision: number) => ({
  subject: { id: "#5", name: "Nick" },
  revision,
  vitals: [
    {
      id: "wod.willpower",
      label: "WP",
      role: "pool",
      value: { kind: "bounded", current: 6, min: 0, max: 10 },
      state: "ok",
    },
    {
      id: "wod.health",
      label: "Health",
      role: "dominant",
      value: { kind: "track", boxes: ["lethal", null], level: "Bruised" },
      state: "warn",
      detail: "-1",
    },
    { id: "wod.wound", label: "wound", role: "readout", value: { kind: "text", text: "0" }, state: "ok" },
    { id: "bad", label: "x", role: "pool", value: { kind: "bounded" }, state: "ok" },
  ],
});

describe("VitalsStore", () => {
  it("parses a frame, drops malformed vitals, and orders dominant first", () => {
    const parsed = parseVitalsFrame(frame(3));
    expect(parsed?.vitals.map((v) => v.id)).toEqual(["wod.willpower", "wod.health", "wod.wound"]);
    expect(orderedVitals(parsed!).map((v) => v.id)).toEqual([
      "wod.health",
      "wod.willpower",
      "wod.wound",
    ]);
    expect(parseVitalsFrame({ subject: {}, revision: 1, vitals: [] })).toBeNull();
    expect(parseVitalsFrame("no")).toBeNull();
  });

  it("ignores a lower revision and keeps groups by feed", () => {
    const s = new VitalsStore();
    expect(s.apply("wod.vitals", frame(3), 10)).toBe(true);
    expect(s.apply("wod.vitals", frame(2), 11)).toBe(false);
    expect(s.groups["wod.vitals"]?.frame.revision).toBe(3);
    expect(s.apply("wod.vitals", frame(4), 12)).toBe(true);
    expect(s.apply("sr.vitals", frame(1), 12)).toBe(true);
    expect(s.list.map((g) => g.feed)).toEqual(["sr.vitals", "wod.vitals"]);
    expect(s.loaded).toBe(true);
    s.clear();
    expect(s.loaded).toBe(false);
  });

  it("gauge helpers", () => {
    const parsed = parseVitalsFrame(frame(1))!;
    const wp = parsed.vitals.find((v) => v.id === "wod.willpower")!;
    expect(vitalPct(wp)).toBe(60);
    expect(vitalPct(parsed.vitals.find((v) => v.id === "wod.wound")!)).toBeNull();
    expect(boxGlyph("lethal")).toBe("X");
    expect(boxGlyph("bashing")).toBe("/");
    expect(boxGlyph("aggravated")).toBe("*");
    expect(boxGlyph("heat")).toBe("H");
    expect(boxGlyph(null)).toBe("");
  });
});
