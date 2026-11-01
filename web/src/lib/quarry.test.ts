// The quarry's frames through the web's decoders (doc 20 cut SM.9; D-20-13: one protocol, three
// renderers). The three frames the Rust side blessed — the viewer's own site view, a second
// principal's, and the Ceres ground chart — decode through `parseSiteFrame` /
// `parseGroundFrame` over the generic grid decoder, and their grid rows are exactly the rows
// each frame's own line-mode print shows (the site map's doubled, the ground chart's one
// character a cell), byte for byte. The pure helpers the screen paints from — the prefills, the
// view/layer/rig steps, a clicked cell's site cell, a place on the 2:1 picture and the leg rule
// that places a moving rig — are pinned to the TUI's and the server's own expectations.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  BODY_GROUND_FEED,
  SITE_MAP_FEED,
  TV_STATUS_FEED,
  groundParams,
  groundPoint,
  groundPrintRows,
  icNow,
  legPosition,
  nextFamily,
  nextMode,
  nextRig,
  parseGroundFrame,
  parseSiteFrame,
  placedRows,
  rigPrefill,
  siteCellOf,
  siteParams,
  type SiteFrame,
} from "./quarry";
import { expandRows } from "./stores/map.svelte";
import { QuarryStore } from "./stores/quarry.svelte";

function fixture(name: string): Record<string, unknown> {
  const path = resolve(__dirname, "fixtures/site", name);
  return JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>;
}

describe("quarry feeds", () => {
  it("names the three feeds as the server does", () => {
    expect([SITE_MAP_FEED, BODY_GROUND_FEED, TV_STATUS_FEED]).toEqual(["site.map", "body.ground", "tv.status"]);
  });
});

describe("body.ground", () => {
  const raw = fixture("body_ground.json");
  const frame = parseGroundFrame(raw)!;

  it("decodes with its ground block", () => {
    expect(frame).not.toBeNull();
    expect(frame.grid.schema.id).toBe("body.ground");
    expect(frame.grid.width).toBe(56);
    expect(frame.grid.height).toBe(28);
    expect(frame.ground?.body).toBe("ceres");
    expect(frame.ground?.family_word).toBe("metals");
    expect(frame.ground?.families).toEqual(["metals", "volatiles", "carbon", "strategic"]);
    expect(frame.ground?.ports.map((p) => p.glyph)).toEqual(["F"]);
    // Doc 32 cut S.1: and Urvara Reach, drawn at its own province.
    expect(frame.ground?.sites.map((s) => s.letter)).toEqual(["A", "C", "J", "K", "O", "U"]);
    expect(frame.ground?.also).toEqual(["C Freeport Commons", "O Occator Rim"]);
    expect(frame.ground?.sites.every((s) => s.best === null)).toBe(true);
    // The screen shows the list's own sentence for the readings, never one of its own.
    expect(frame.ground?.sites.every((s) => s.best_line === "Your readings: none.")).toBe(true);
    expect(
      frame.ground?.sites.every((s) => frame.ground?.lines.includes(`    ${s.best_line}`)),
    ).toBe(true);
  });

  it("draws byte-identically to its own print, north up", () => {
    const decoded = expandRows(frame.grid).map((row) => row.join("")).reverse();
    expect(decoded).toEqual(groundPrintRows(frame.ground!, 28));
    expect(decoded.every((row) => row.length === 56)).toBe(true);
  });

  it("places a site on the 2:1 picture, north at the top", () => {
    const ahuna = frame.ground!.sites.find((s) => s.letter === "A")!;
    const p = groundPoint(ahuna, 560, 280);
    expect(p.x).toBeCloseTo((ahuna.lon_mdeg / 360_000) * 560, 6);
    expect(Math.floor(ahuna.lon_mdeg / 100)).toBe(3162);
    expect(p.y).toBeGreaterThan(140);
    const freeport = groundPoint(frame.ground!.ports[0]!, 560, 280);
    expect(freeport.y).toBeLessThan(140);
  });
});

describe("site.map", () => {
  for (const [name, mine] of [
    ["site_map.json", true],
    ["site_map_other.json", false],
  ] as const) {
    it(`${name} decodes with its site block and draws its own print`, () => {
      const raw = fixture(name) as { height: number; site: { print: string[] } };
      const frame = parseSiteFrame(raw)!;
      expect(frame.site).not.toBeNull();
      const rows = raw.site.print
        .slice(3, 3 + raw.height)
        .map((line) => [...line].filter((_, i) => i % 2 === 0).join(""))
        .reverse();
      expect(placedRows(frame, null).map((r) => r.join(""))).toEqual(rows);
      // At the frame's own instant the placed rows are the frame's rows.
      expect(placedRows(frame, frame.site!.clock?.now_ic ?? 0).map((r) => r.join(""))).toEqual(rows);
      expect(frame.site!.families).toEqual(["metals", "volatiles", "carbon", "strategic"]);
      expect(frame.site!.addressed).toBe(mine ? 1 : null);
      expect(frame.grid.legend.map(([g]) => g)).toContain("V");
      // The side pane's two sentences ride as keys, whole (the pre-commit round, F10): the
      // viewer's frame reads pockets and sees no other rig; the second principal's reads none,
      // and — with no rig of its own on the ground (doc 32 cut CF.4: exact cells only within
      // sight) — sees the viewer's rig as a presence band, never an `Other rigs:` distance.
      expect(frame.site!.other_rigs).toBeNull();
      if (mine) {
        expect(frame.site!.no_readings).toBeNull();
      } else {
        expect(frame.site!.no_readings).toBe("You have no readings of metals in view.");
        expect(frame.site!.lines).toContain(frame.site!.no_readings);
        expect(frame.site!.lines).toContain("Presence: N 200-500 m.");
      }
      if (mine) {
        expect(frame.site!.readings.every((r) => r.line.startsWith(r.text))).toBe(true);
        expect(frame.site!.readings.every((r) => r.text.startsWith(r.target))).toBe(true);
        expect(frame.site!.log.length).toBeGreaterThan(0);
      } else {
        expect(frame.site!.log).toEqual([]);
      }
    });
  }

  it("decodes the two sentences tolerantly: absent or not a string is null", () => {
    const raw = fixture("site_map_other.json") as { site: Record<string, unknown> };
    delete raw.site.no_readings;
    raw.site.other_rigs = 7;
    const frame = parseSiteFrame(raw)!;
    expect(frame.site!.no_readings).toBeNull();
    expect(frame.site!.other_rigs).toBeNull();
  });

  it("maps a clicked frame cell to the site cell it drives to", () => {
    const frame = parseSiteFrame(fixture("site_map.json"))!;
    expect(frame.site!.grid).toEqual({ x0: 0, y0: 0, span: 8, cell_cm: 1000 });
    expect(siteCellOf(frame.site!, 18, 22)).toEqual([148, 180]);
  });
});

// The pre-commit round (the judge's correctness/F7): the store's pick stands only while the
// held frame lists it; the next drive-view ask then carries no rig.
describe("the quarry store's rig pick", () => {
  const two = (): Record<string, unknown> => {
    const raw = fixture("site_map.json") as { site: { rigs: Record<string, unknown>[] } };
    raw.site.rigs.push({ ...raw.site.rigs[0]!, slot: 2, line: "Rig 2: in the bay." });
    return raw;
  };

  it("clears a pick the applied frame no longer lists", () => {
    const store = new QuarryStore();
    store.applySite(two(), 0);
    store.stepRig();
    expect(store.rig).toBe(2);
    store.applySite(two(), 1);
    expect(store.rig).toBe(2);
    expect(siteParams(store.mode, store.rig, store.family)).toEqual({ view: "drive", rig: 2 });
    // Rig 2 returned: the frame lists rig 1 alone.
    store.applySite(fixture("site_map.json"), 2);
    expect(store.rig).toBeNull();
    expect(siteParams(store.mode, store.rig, store.family)).toEqual({ view: "drive" });
    expect(store.selectedRig).toBe(1);
  });

  it("clears a pick on a reason frame, and keeps it on a frame that does not decode", () => {
    const store = new QuarryStore();
    store.applySite(two(), 0);
    store.stepRig();
    expect(store.rig).toBe(2);
    store.applySite({ schema: { id: "site.map", shape: "status" } }, 1);
    expect(store.refused).toBe(true);
    expect(store.rig).toBe(2);
    const reason = {
      schema: { id: "site.map", shape: "grid", version: 1 },
      window: 0,
      title: "Rig 2 is not on Ahuna Mons's ground or in the bay.",
      width: 0,
      height: 0,
      rows: [],
      legend: [],
      units: [],
      you: null,
    };
    // Doc 20 cut SM.10: a reason frame that clears a pick says so (the caller re-asks once);
    // the same frame again clears nothing and says nothing.
    expect(store.applySite(reason, 2)).toBe(true);
    expect(store.rig).toBeNull();
    expect(store.applySite(reason, 3)).toBe(false);
    expect(siteParams(store.mode, store.rig, store.family)).toEqual({ view: "drive" });
  });
});

describe("the keys' rules", () => {
  it("prefills, never sends", () => {
    expect(rigPrefill(1, "scan")).toBe("+tv/1 scan");
    expect(rigPrefill(2, "drive ")).toBe("+tv/2 drive ");
    expect(rigPrefill(null, "mine")).toBe("+tv mine");
  });

  it("steps the rig, the family and the view as the TUI does", () => {
    expect(nextRig([2, 1], null)).toBe(1);
    expect(nextRig([1, 2], 1)).toBe(2);
    expect(nextRig([1, 2], 2)).toBe(1);
    expect(nextRig([], 1)).toBeNull();
    const words = ["metals", "volatiles", "carbon", "strategic"];
    expect(nextFamily(words, "carbon")).toBe("strategic");
    expect(nextFamily(words, "strategic")).toBe("metals");
    expect(nextFamily(words, null)).toBe("metals");
    expect(nextMode("drive")).toBe("site");
    expect(nextMode("site")).toBe("ground");
    expect(nextMode("ground")).toBe("drive");
  });

  it("shapes the parameters the TUI sends", () => {
    expect(siteParams("drive", null, null)).toEqual({ view: "drive" });
    expect(siteParams("ground", 1, "metals")).toEqual({ view: "site", rig: 1, family: "metals" });
    expect(groundParams(null)).toEqual({});
    expect(groundParams("ice")).toEqual({ family: "ice" });
  });
});

describe("a moving rig", () => {
  it("follows the server's leg rule (its own unit test's expectations)", () => {
    const leg = [
      { x_cm: 0, y_cm: 0, at_s: 0 },
      { x_cm: 10_000, y_cm: 0, at_s: 100 },
      { x_cm: 10_000, y_cm: 20_000, at_s: 300 },
    ];
    expect(legPosition(leg, -5)).toEqual([0, 0]);
    expect(legPosition(leg, 50)).toEqual([5_000, 0]);
    expect(legPosition(leg, 100)).toEqual([10_000, 0]);
    expect(legPosition(leg, 200)).toEqual([10_000, 10_000]);
    expect(legPosition(leg, 9_999)).toEqual([10_000, 20_000]);
    expect(legPosition([], 5)).toBeNull();
  });

  it("is placed along its leg, the cell it left restored from the frame", () => {
    const raw = {
      schema: { id: "site.map", shape: "grid", version: 1 },
      window: 0,
      width: 4,
      height: 1,
      rows: [[["@", 1], [",", 2], ["^", 1]]],
      legend: [],
      units: [],
      you: null,
      site: {
        grid: { x0: 10, y0: 20, span: 1, cell_cm: 1000 },
        under: [[0, 0, "."]],
        rigs: [
          {
            slot: 1,
            cell: [10, 20],
            leg: {
              started_ic: 500,
              waypoints: [
                { x_cm: 10_500, y_cm: 20_500, at_s: 0 },
                { x_cm: 13_500, y_cm: 20_500, at_s: 30 },
              ],
            },
          },
        ],
        clock: { now_ic: 500, rate: 1 },
      },
    };
    const frame = parseSiteFrame(raw) as SiteFrame;
    const text = (ic: number | null) => placedRows(frame, ic).map((r) => r.join("")).join("");
    expect(text(500)).toBe("@,,^");
    expect(text(null)).toBe("@,,^");
    expect(text(520)).toBe(".,@^");
    expect(text(900)).toBe(".,,@");
    expect(icNow(frame.site!, 1_000, 8_400)).toBe(507);
  });

  // The server's compose counts only the viewer's rigs inside the window: a rig on the ground
  // past the window's edge draws no `@` and lends no other rig a slot digit (the TUI's
  // `sm9_a_rig_outside_the_window_lends_no_slot_digit`, the same frame and answers).
  it("counts only the rigs inside the window for the slot digits", () => {
    const raw = {
      schema: { id: "site.map", shape: "grid", version: 1 },
      window: 0,
      width: 4,
      height: 1,
      rows: [[[",", 1], ["@", 1], [".", 1], ["^", 1]]],
      legend: [],
      units: [],
      you: null,
      site: {
        grid: { x0: 10, y0: 20, span: 1, cell_cm: 1000 },
        under: [[1, 0, ","]],
        rigs: [
          { slot: 1, cell: [11, 20], leg: null },
          {
            slot: 2,
            cell: [40, 20],
            leg: {
              started_ic: 500,
              waypoints: [
                { x_cm: 40_500, y_cm: 20_500, at_s: 0 },
                { x_cm: 13_500, y_cm: 20_500, at_s: 270 },
              ],
            },
          },
        ],
        clock: { now_ic: 500, rate: 1 },
      },
    };
    const frame = parseSiteFrame(raw) as SiteFrame;
    const text = (ic: number | null) => placedRows(frame, ic).map((r) => r.join("")).join("");
    expect(text(500)).toBe(",@.^");
    expect(text(770)).toBe(",@1@");
  });
});

describe("a rig's run and queue", () => {
  // The record-backed rig states the screen shows under a rig's line: a drilling rig's run
  // clause (from its operator's projection) and a queued unload's line (doc 20 cut SM.8's
  // queue: in the bay, unload queued — never idle).
  it("decodes the run and the queued unload", () => {
    const raw = fixture("site_map.json") as { site: { rigs: Record<string, unknown>[] } };
    const variant = structuredClone(raw);
    variant.site.rigs[0]!.queued = "Unloads its hopper into the hold next.";
    variant.site.rigs[0]!.run = {
      addr: "G71",
      ore: "metal ore",
      k: 1,
      cycles: 4,
      due_ic: 12_000,
      text: "drilling G71 for metal ore, cycle 1 of 4, this cycle done in 5 min",
    };
    const frame = parseSiteFrame(variant)!;
    const rig = frame.site!.rigs[0]!;
    expect(rig.queued).toBe("Unloads its hopper into the hold next.");
    expect(rig.run?.text).toBe("drilling G71 for metal ore, cycle 1 of 4, this cycle done in 5 min");
    expect(rig.run?.stop).toBe(false);
    expect(rig.run?.blind).toBe(false);
    // The blessed frame's rig has neither.
    const blessed = parseSiteFrame(raw)!.site!.rigs[0]!;
    expect(blessed.queued).toBeNull();
    expect(blessed.run).toBeNull();
  });

  // Doc 20 cut SM.11: a rig's fill plan, on its author's frame (step words and hours only).
  it("decodes the rig's plan", () => {
    const raw = fixture("site_map.json") as { site: { rigs: Record<string, unknown>[] } };
    const variant = structuredClone(raw);
    variant.site.rigs[0]!.plan = {
      no: 2,
      state: "live",
      step: 3,
      steps: 7,
      next: ["mine C118", "sound D-NW", 7],
      deadline_ic: 99_000,
      text: "Plan 2: step 3 of 7, mine C118",
    };
    const plan = parseSiteFrame(variant)!.site!.rigs[0]!.plan!;
    expect(plan.text).toBe("Plan 2: step 3 of 7, mine C118");
    expect(plan.next).toEqual(["mine C118", "sound D-NW"]);
    expect(plan.end).toBeNull();
    expect(parseSiteFrame(raw)!.site!.rigs[0]!.plan).toBeNull();
  });
});
