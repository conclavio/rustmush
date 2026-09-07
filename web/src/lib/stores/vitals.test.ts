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

  it("replaces on a new subject or a reset watermark (interface cut I8)", () => {
    // One feed can follow a viewer from subject to subject, and a subject
    // recreated in place keeps its id while its revision resets to 0.
    // Neither may be dropped as stale, or the group latches forever.
    const other = (revision: number) => ({
      ...frame(revision),
      subject: { id: "#9", name: "Other" },
    });
    const s = new VitalsStore();
    expect(s.apply("tactics.vitals", frame(7), 10)).toBe(true);
    // A lower revision naming a DIFFERENT subject replaces.
    expect(s.apply("tactics.vitals", other(2), 11)).toBe(true);
    expect(s.groups["tactics.vitals"]?.frame.subject.id).toBe("#9");
    expect(s.groups["tactics.vitals"]?.frame.revision).toBe(2);
    // Revision 0 with the SAME subject (recreated in place) replaces.
    expect(s.apply("tactics.vitals", other(0), 12)).toBe(true);
    expect(s.groups["tactics.vitals"]?.frame.revision).toBe(0);
    // A lower non-zero revision with the same subject is still dropped.
    expect(s.apply("tactics.vitals", other(5), 13)).toBe(true);
    expect(s.apply("tactics.vitals", other(4), 14)).toBe(false);
    expect(s.groups["tactics.vitals"]?.frame.revision).toBe(5);
  });

  it("a null keyframe drops the feed's group (interface cut I8)", () => {
    // `data: null` is the feed saying there is no subject — an answer, not
    // a malformed frame. Without this arm a dead or de-stationed subject's
    // gauges stay on the strip forever, presented as live.
    const s = new VitalsStore();
    expect(s.apply("tactics.vitals", frame(3), 10)).toBe(true);
    expect(s.apply("wod.vitals", frame(3), 10)).toBe(true);
    expect(s.apply("tactics.vitals", null, 11)).toBe(true);
    expect(s.groups["tactics.vitals"]).toBeUndefined();
    // Another feed's group is untouched.
    expect(s.groups["wod.vitals"]?.frame.revision).toBe(3);
    expect(s.loaded).toBe(true);
    // A null on a feed nothing was held for is a no-op that still answers.
    expect(s.apply("sr.vitals", null, 12)).toBe(true);
    expect(s.list.map((g) => g.feed)).toEqual(["wod.vitals"]);
    // A malformed NON-null payload is still a refusal that keeps the group.
    expect(s.apply("wod.vitals", { subject: {}, revision: 1, vitals: [] }, 13)).toBe(false);
    expect(s.apply("wod.vitals", "nope", 13)).toBe(false);
    expect(s.groups["wod.vitals"]?.frame.revision).toBe(3);
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
