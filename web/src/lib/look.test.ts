// A unit's look in the web client (doc 22 cut U.4): the doll grid read from
// the same files the gateway reads, what each part shows, the tones that tell
// touching parts apart, and the colour ramp.

import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { decodeCockpitFrame } from "./cockpit";
import { dollColour, dollCondition, dollTones, LOOKS, lookFor, parseDoll } from "./look";

const LOOK_DIR = resolve(process.cwd(), "../crates/mush-pixel/looks");
const fixture = JSON.parse(
  readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/cockpit_frame.json"), "utf8"),
) as unknown;

describe("the shipped looks", () => {
  it("are the gateway's own files, every one read", () => {
    const files = readdirSync(LOOK_DIR).filter((f) => f.endsWith(".json"));
    expect(LOOKS).toHaveLength(files.length);
    expect(lookFor("tacsim.unit.line_tank")?.doll?.view).toBe("top");
    expect(lookFor("tacsim.unit.biped_mk2")?.doll?.view).toBe("upright");
    expect(lookFor("tacsim.unit.dray")).toBeNull();
    expect(lookFor(null)).toBeNull();
    expect(lookFor("unknown")?.doll).toBeNull();
  });

  it("refuses a doll that does not hold together", () => {
    expect(parseDoll({ view: "top", rows: ["ab", "a"], legend: { a: { section: "s" }, b: { section: "t" } } })).toBeNull();
    expect(parseDoll({ view: "side", rows: ["a"], legend: { a: { section: "s" } } })).toBeNull();
    expect(parseDoll({ view: "top", rows: ["ax"], legend: { a: { section: "s" } } })).toBeNull();
    const ok = parseDoll({ view: "top", rows: ["b.a"], legend: { b: { section: "t" }, a: { section: "s", facet: "front" } } });
    // The legend's characters in sorted order, as the Rust reader takes them.
    expect(ok?.parts).toEqual([
      { section: "s", facet: "front" },
      { section: "t", facet: null },
    ]);
    expect(ok?.cells).toEqual([1, null, 0]);
  });
});

describe("the tank's doll on the fixture frame", () => {
  const frame = decodeCockpitFrame(fixture)!;
  const doll = lookFor(frame.unit_type)!.doll!;

  it("reads the frame's own unit type", () => {
    expect(frame.unit_type).toBe("tacsim.unit.line_tank");
  });

  it("shows a facet's armour, else the section's internals, none once destroyed", () => {
    const cond = dollCondition(doll, frame.sections);
    const of = (section: string, facet: string | null) =>
      cond[doll.parts.findIndex((p) => p.section === section && p.facet === facet)];
    expect(of("hull", "front")).toEqual([55, 80]);
    expect(of("hull", "left")).toEqual([60, 60]);
    expect(of("right_track", null)).toEqual([0, 24]);
    expect(of("left_track", null)).toEqual([6, 24]);
  });

  it("tells touching parts apart by tone", () => {
    const tones = dollTones(doll);
    const tone = (section: string, facet: string | null) =>
      tones[doll.parts.findIndex((p) => p.section === section && p.facet === facet)];
    expect(tone("hull", null)).not.toBe(tone("turret", null));
    expect(tone("hull", null)).not.toBe(tone("hull", "front"));
    expect(tone("hull", null)).not.toBe(tone("engine", null));
  });
});

describe("the colour ramp", () => {
  it("is green, amber, red, grey when gone, lighter at tone 1", () => {
    expect(dollColour(60, 60, 0)).toBe("#46be5a");
    expect(dollColour(40, 80, 0)).toBe("#e6b432");
    expect(dollColour(6, 24, 0)).toBe("#dc3c32");
    expect(dollColour(0, 24, 0)).toBe("#4a4a52");
    expect(dollColour(60, 60, 1)).not.toBe(dollColour(60, 60, 0));
  });
});
