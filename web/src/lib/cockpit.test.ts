// The cockpit's pure half (16b cut Q.5; doc 16 §5): the two decoders, the ONE
// map composition, and the view model the screen prints.
//
// The load-bearing assertion here is `the parity golden`: this client's
// composition of `fixtures/cockpit/arena_map.json` + `cockpit_frame.json` at
// exactly 39 by 20 cells equals `view_grid.txt` byte for byte — the same file the
// TUI's `tui_golden.rs` and `mush-ansi`'s `golden.rs` are held to (16b cut B.4:
// the file is one glyph per cell; the line client prints each cell twice, and
// `golden.rs` halves its printed rows to compare). Three
// renderers each pinned to a golden of its own would be regression; one file
// all three must reproduce is parity.
//
// The fixtures are READ, never copied. Re-bless the view-model golden this cut
// adds with `COCKPIT_BLESS=1 npx vitest run src/lib/cockpit.test.ts`; the three
// JSON files and `view_grid.txt` are blessed on the Rust side (`TACSIM_BLESS=1`).

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import {
  armingWords,
  catchupLines,
  cockpitView,
  cockpitViewLines,
  composeWindow,
  contactRows,
  contactText,
  damageText,
  decodeArenaMap,
  decodeCatchup,
  decodeCockpitFrame,
  decodeMind,
  decodeRules,
  labelledAt,
  labelPlan,
  markerLegend,
  MAX_MIND_PROFILES,
  mindLines,
  mindProfiles,
  noticeLine,
  noticeLines,
  ruleSummaryLines,
  scanChips,
  scanText,
  sideWord,
  statusLines,
  weaponLines,
  type ArenaMap,
  type CockpitArming,
  type CockpitFrame,
  type CockpitMind,
  type CockpitNotice,
  type Marker,
} from "./cockpit";

const DIR = "src/lib/fixtures/cockpit";

function fixture(name: string): string {
  return readFileSync(resolve(process.cwd(), DIR, name), "utf8");
}

function json(name: string): unknown {
  return JSON.parse(fixture(name)) as unknown;
}

function theFrame(): CockpitFrame {
  const frame = decodeCockpitFrame(json("cockpit_frame.json"));
  if (frame === null) {
    throw new Error("the committed cockpit frame did not decode");
  }
  return frame;
}

function theMap(): ArenaMap {
  const map = decodeArenaMap(json("arena_map.json"));
  if (map === null) {
    throw new Error("the committed arena map did not decode");
  }
  return map;
}

describe("ordered rule summaries", () => {
  const rule = (slot: number, action: string) => ({ slot, rule: { action }, text: "full terms" });

  it("shows every slot, including holds and Sensors rules, in order at the pane floor", () => {
    const frame = decodeCockpitFrame({
      rules: [rule(6, "fire"), rule(1, "fire"), rule(2, "hold_fire"),
        rule(3, "lock"), rule(4, "fire"), rule(5, "hold_fire")],
    })!;
    const lines = ruleSummaryLines(frame);
    expect(lines).toEqual([
      "rules: S1 fire S2 hold S3 lock",
      "S4 fire S5 hold S6 fire; MIND RULES",
    ]);
    expect(lines.every((line) => [...line].length <= 44)).toBe(true);
    expect(weaponLines(frame)).toEqual([...lines, "(no weapon)"]);
    expect(ruleSummaryLines({ ...frame, seated: false })).toEqual([]);

    const sparse = decodeCockpitFrame({
      rules: [rule(1, "fire"), rule(3, "hold_fire"), rule(6, "lock")],
    })!;
    expect(ruleSummaryLines(sparse)).toEqual(["rules: S1 fire S3 hold S6 lock; MIND RULES"]);
    expect(ruleSummaryLines(sparse)[0]!.length).toBeLessThanOrEqual(44);

    const unarmed = { ...sparse, mind: {
      name: "Standard Doctrine", profiles: ["Line"], active: 0, card: null, sealedRules: [],
      class: "I", core: "II", channels: ["helm", "sensors"],
      next_switch_ms: null, off: false,
    } };
    expect(cockpitView(unarmed, null, []).weapons).toEqual(ruleSummaryLines(sparse));
  });

  it("rejects invalid slots, unknown actions and duplicates without retaining raw terms", () => {
    expect(decodeRules(null)).toEqual([]);
    expect(decodeRules({})).toEqual([]);
    expect(decodeRules([
      null, {}, rule(0, "fire"), rule(7, "fire"), rule(1.5, "fire"),
      rule(1, "unknown"), rule(2, "hold"), rule(1, "fire"), rule(1, "lock"),
      { slot: 3, rule: { action: "lock", contact: "ffffffffffffffff" }, text: "\u001bhidden" },
    ])).toEqual([{ slot: 1, action: "fire" }, { slot: 3, action: "lock" }]);
    const old = decodeCockpitFrame({})!;
    expect(old.rules).toEqual([]);
    expect(weaponLines(old)).toEqual(["(no weapon)"]);
  });

  it("uses canonical rules instead of an incomplete first-fire convenience line", () => {
    const weapon = { id: "cannon", ready: true, rounds: 14, rounds_max: 20,
      armed: { slot: 1, target: "lock", min_permille: 100, aspects: [], keeps: 0 } };
    const current = decodeCockpitFrame({ weapons: [weapon], rules: [rule(1, "fire")] })!;
    expect(weaponLines(current)).toEqual(["rules: S1 fire; MIND RULES", "cannon  ready  14/20"]);
    const older = decodeCockpitFrame({ weapons: [weapon] })!;
    expect(weaponLines(older)).toHaveLength(2);
    expect(weaponLines(older)[1]).toContain("S1:");
  });

  it("decodes movement from the held plot as a strict boolean", () => {
    const frame = decodeCockpitFrame({
      contacts: [{ moving: true }, { moving: false }, {}, { moving: "true" }],
    })!;
    expect(frame.contacts.map((contact) => contact.moving)).toEqual([true, false, false, false]);
  });
});

describe("the decoders", () => {
  it("read the committed cockpit frame", () => {
    const frame = theFrame();
    expect(frame.seated).toBe(true);
    expect(frame.match).toBe(7);
    expect(frame.callsign).toBe("RA");
    expect(frame.cell).toEqual([6, 4]);
    expect(frame.power).toBe("online");
    expect(frame.blocked).toBe(false);
    expect(frame.speed_kph).toBe(43);
    expect(frame.heading_deg).toBe(45);
    expect(frame.heat).toEqual({ heat: 40, max: 100, overheated: false });
    expect(frame.tempo).toBe("engaged");
    // 16b cut D.2: the pilot's own two limits and this platform's own scope.
    expect(frame.scope).toBe("hostile");
    expect(frame.wreck).toBe(false);
    expect(frame.immobile).toBe(false);
    // 16b cut Y.4: an ordinary pulse carries no catch-up report.
    expect(frame.catchup).toBeNull();
    expect(frame.lock_designator).toBe("BA");
    // 16b cut D.4 (16c `T-20`): derived from the locked row's own plot age.
    expect(frame.lock_range_m).toBe(440);
    expect(frame.lock_percent).toBe(62);
    expect(frame.contacts.map((c) => c.designator)).toEqual(["BA", "RB", "X3", "X4"]);
    expect(frame.contacts[0]?.condition).toEqual({ table: 0, bands: "ggyrd", age_ms: 2000 });
    expect(frame.contacts[1]?.condition).toBeNull();
    expect(frame.scan_tables).toEqual([
      ["engine", "hull", "left_track", "right_track", "turret"],
    ]);
    expect(frame.sections.map((s) => s.id)).toEqual([
      "engine",
      "hull",
      "left_track",
      "right_track",
      "turret",
    ]);
    expect(frame.weapons.map((w) => w.id)).toEqual(["cannon", "machine_gun"]);
    expect(frame.weapons[1]?.percent).toBe(21);
    expect(frame.weapons.map((w) => w.heat_per_shot)).toEqual([0, 0]);
    expect(frame.notices.map((n) => n.kind)).toEqual(["acquired", "target_section_lost"]);
    expect(frame.notices[1]?.section).toBe("turret");
  });

  it("keep NOTHING the observer rule forbids: no id, no lock digits, no unit, no text", () => {
    const frame = theFrame();
    const flat = JSON.stringify(frame);
    // The sixteen-digit contact ids, the lock's copy of one, and the object
    // number are all in the payload and none of them survives the decode.
    expect(flat).not.toMatch(/[0-9a-f]{16}/);
    expect(flat).not.toContain("New contact X4");
    expect(flat).not.toContain("turret is destroyed");
    for (const key of ["id", "lock", "unit", "text"]) {
      expect(Object.keys(frame)).not.toContain(key);
    }
    for (const contact of frame.contacts) {
      expect(Object.keys(contact)).not.toContain("id");
    }
    for (const notice of frame.notices) {
      expect(Object.keys(notice)).not.toContain("text");
    }
  });

  it("read the committed arena map", () => {
    const map = theMap();
    expect(map.seated).toBe(true);
    expect(map.match).toBe(7);
    expect(map.surface).toBe("fixture_yard");
    expect(map.cell_cm).toBe(1000);
    expect(map.width).toBe(96);
    expect(map.height).toBe(64);
    expect(map.origin_cell).toEqual([0, 0]);
    expect(map.rows).toHaveLength(64);
    expect(map.rows[0]).toEqual([['"', 96]]);
    expect(map.legend[0]).toEqual(['"', "grass"]);
    // 16b cut V.1: the ground's three named spots, in authored order.
    expect(map.spots).toEqual([
      { letter: "A", name: "The Ford", cell: [25, 3] },
      { letter: "B", name: "The Roof", cell: [14, 7] },
      { letter: "C", name: "The Woods", cell: [50, 25] },
    ]);
  });

  it("drops a malformed spot and keeps the rest (16b cut V.1)", () => {
    const raw = json("arena_map.json") as Record<string, unknown>;
    const map = decodeArenaMap({
      ...raw,
      spots: [
        { letter: "A", name: "The Ford", cell: [25, 3] },
        { letter: "AB", name: "Two Letters", cell: [1, 1] },
        { letter: "", name: "No Letter", cell: [1, 1] },
        { letter: "\u0007", name: "A Bell", cell: [1, 1] },
        { name: "No Letter Key", cell: [1, 1] },
        { letter: "E", cell: [1, 1] },
        { letter: "F", name: "No Cell" },
        { letter: "G", name: "Bad Cell", cell: [-1, 2] },
        "junk",
        { letter: "H", name: `Long ${"n".repeat(60)}`, cell: [2, 2] },
      ],
    });
    expect(map?.spots.map((s) => s.letter)).toEqual(["A", "H"]);
    // Cleaned and cut to forty, as the server bounds a name at load.
    expect([...(map?.spots[1]?.name ?? "")]).toHaveLength(40);
    // No `spots` key at all is no spots.
    const older = { ...raw };
    delete older.spots;
    expect(decodeArenaMap(older)?.spots).toEqual([]);
    expect(decodeArenaMap({ ...raw, spots: "the ford" })?.spots).toEqual([]);
  });

  it("decode the VIEW composite as one more status frame, its grid ignored", () => {
    const raw = json("view_frame.json") as Record<string, unknown>;
    expect((raw.schema as Record<string, unknown>).shape).toBe("grid");
    const frame = decodeCockpitFrame(raw);
    expect(frame).not.toBeNull();
    expect(frame?.seated).toBe(true);
    expect(frame?.match).toBe(7);
    expect(frame?.callsign).toBe("RA");
    expect(frame?.contacts.map((c) => c.designator)).toEqual(["BA", "RB", "X3", "X4"]);
    // The window's own keys are not read: this client composes its own.
    expect(Object.keys(frame ?? {})).not.toContain("rows");
    expect(Object.keys(frame ?? {})).not.toContain("units");
    expect(Object.keys(frame ?? {})).not.toContain("you");
  });

  it("tolerate a missing key, an unknown key and seated: false", () => {
    // An older server: no `scan_tables`, no `condition`, no `lock_*`.
    const old = decodeCockpitFrame({
      match: 3,
      callsign: "BB",
      contacts: [{ designator: "X1", class: "unknown" }],
      sections: [{ id: "hull" }],
      weapons: [{ id: "gun" }],
      notices: [{ kind: "acquired" }],
    });
    expect(old?.seated).toBe(true);
    expect(old?.scan_tables).toEqual([]);
    expect(old?.contacts[0]?.condition).toBeNull();
    expect(old?.contacts[0]?.cell).toBeNull();
    expect(old?.lock_designator).toBeNull();
    expect(old?.heat).toBeNull();
    expect(old?.sections[0]?.internals).toBe(0);
    expect(old?.weapons[0]?.percent).toBeNull();
    // 16b cut Y.4: a server that predates the catch-up sends no key.
    expect(old?.catchup).toBeNull();
    // A newer server: a field this build predates is ignored, not fatal.
    const ahead = decodeCockpitFrame({
      ...(json("cockpit_frame.json") as Record<string, unknown>),
      ammo_bins: [{ id: "left", rounds: 9 }],
      schema: { id: "arena.cockpit", version: 2, retain: "latest" },
    });
    expect(ahead?.callsign).toBe("RA");
    expect(Object.keys(ahead ?? {})).not.toContain("ammo_bins");
    // The cleared frame, which is the LAST one a pilot gets.
    const cleared = decodeCockpitFrame({
      schema: { id: "arena.cockpit", version: 1, retain: "latest" },
      seated: false,
    });
    expect(cleared?.seated).toBe(false);
    expect(cleared?.match).toBeNull();
    // Not an object at all.
    expect(decodeCockpitFrame(null)).toBeNull();
    expect(decodeCockpitFrame([1, 2])).toBeNull();
    expect(decodeCockpitFrame("frame")).toBeNull();
    // The map's own tolerance, and its unseated answer.
    const unseated = decodeArenaMap({
      schema: { id: "arena.map", version: 1, shape: "terrain", retain: "latest" },
      seated: false,
    });
    expect(unseated?.seated).toBe(false);
    expect(unseated?.width).toBe(0);
    expect(decodeArenaMap(undefined)).toBeNull();
    const odd = decodeArenaMap({ width: 2, height: 1, rows: [[["#", 2], "junk"]], legend: "no" });
    expect(odd?.rows).toEqual([[["#", 2]]]);
    expect(odd?.legend).toEqual([]);
    expect(odd?.spots).toEqual([]);
  });

  it("strip control characters and cut each string to its own length", () => {
    // `clean` is the one gate between a server string and a pane border, a
    // legend row or the composer — and every committed fixture is well formed,
    // so without a case of its own nothing here would notice if it stopped
    // working. C0, DEL and C1 all go; an empty or all-control string takes the
    // field's fallback rather than printing blank; designators and callsigns
    // are cut to eight and ids and section words to twenty-four.
    const frame = decodeCockpitFrame({
      match: 5,
      callsign: "R\u0007A",
      power: "onl\u001bine",
      tempo: "   ",
      // 16b cut D.2 (`T-59`, the panel half): `scope` is the word the status
      // block prints now, so it is gated like every other printed string.
      scope: "\u0007\u0007",
      contacts: [{ designator: "BA\u0000DDESIGNATOR", class: "", band: "effe\u009fctive" }],
      sections: [{ id: "left_track_and_then_some_more", condition: "\u007f" }],
      // 16b cut D.2 (`B-16`): a negative per-shot heat would make a hot gun of a
      // cold one at the `FIRE` pre-check on the other side.
      weapons: [{ id: "gun", heat_per_shot: -4 }],
      // 16b cut D.2 (`B-07`): the seven new notice keys are server strings too.
      notices: [
        { kind: "hit", designator: "X\u00019", section: "tur\u0000ret" },
        {
          kind: "ref\u0000used",
          cause: "\u0007",
          refusal: "over\u001bheated_and_then_some_more_words",
          order: "fire_and_then_some_more",
          weapon: "mach\u0000ine_gun_and_then_some_more",
          detail: "boom",
          friendly: "yes",
          spent: 1,
        },
        {
          kind: "refused",
          order: "fire",
          refusal: "cycling",
          detail: { secs: "x", heat: 60.9, max: 60 },
        },
      ],
    });
    expect(frame?.callsign).toBe("RA");
    expect(frame?.power).toBe("online");
    expect(frame?.tempo).toBe("?");
    expect(frame?.scope).toBe("?");
    expect(frame?.weapons[0]?.heat_per_shot).toBe(0);
    expect(frame?.contacts[0]?.class).toBe("?");
    expect(frame?.contacts[0]?.band).toBe("effective");
    expect(frame?.contacts[0]?.designator).toBe("BADDESIG");
    expect(frame?.sections[0]?.condition).toBe("?");
    expect(frame?.sections[0]?.id).toBe("left_track_and_then_some");
    expect(frame?.notices[0]).toEqual({
      kind: "hit",
      designator: "X9",
      was: null,
      section: "turret",
      cause: null,
      refusal: null,
      order: null,
      weapon: null,
      detail: null,
      friendly: false,
      spent: false,
    });
    // Every one of the seven is bounded, and a `detail` that is not an object at
    // all is no detail. A `friendly` or a `spent` that is not `true` is false:
    // the panel's mark is never turned on by a truthy string or a 1.
    expect(frame?.notices[1]).toEqual({
      kind: "refused",
      designator: null,
      was: null,
      section: null,
      cause: null,
      refusal: "overheated_and_then_some",
      order: "fire_and_then_so",
      weapon: "machine_gun_and_then_som",
      detail: null,
      friendly: false,
      spent: false,
    });
    expect(frame?.notices[2]?.detail).toEqual({ secs: null, heat: 60, max: 60 });
    // …and nothing the screen goes on to PRINT carries one either.
    for (const line of cockpitViewLines(
      cockpitView(frame!, null, noticeLines(frame!.notices)),
    )) {
      expect(line).not.toMatch(/[\u0000-\u001f\u007f-\u009f]/);
    }
  });
});

describe("composeWindow", () => {
  // **The parity assertion** (doc 16 §5). If this ever fails, the first
  // differing cell is a defect in one of the three compositors (or in the
  // fixture), not a golden to re-bless.
  it("composes the committed grid at exactly 39 by 20 cells", () => {
    const win = composeWindow(theMap(), theFrame(), 39, 20);
    expect(win).toMatchObject({ x0: 0, y0: 0, w: 39, h: 20 });
    const expected = fixture("view_grid.txt").replace(/\r\n/g, "\n").replace(/\n$/, "");
    const got = win.grid.join("\n");
    if (got !== expected) {
      const want = expected.split("\n");
      const have = got.split("\n");
      for (let i = 0; i < Math.max(want.length, have.length); i += 1) {
        expect(have[i], `row ${i} (north-up) differs; that is a compositor defect`).toBe(want[i]);
      }
    }
    expect(got).toBe(expected);
    // The markers, read off the SOUTH the way the payload gives them.
    const at = (x: number, y: number): string => win.grid[win.h - 1 - y]?.charAt(x) ?? "";
    expect(at(6, 4)).toBe("@");
    expect(at(36, 14)).toBe("x");
    expect(at(2, 9)).toBe("+");
    expect(at(14, 7)).toBe("?"); // beats the building — and the spot B — it stands on
    expect(at(13, 7)).toBe("#");
    // 16b cut V.1: the spot A over the pond.
    expect(at(25, 3)).toBe("A");
    // The marker-free rows keep the building under the contact.
    expect(win.terrain[win.h - 1 - 7]?.charAt(14)).toBe("#");
    // X4 at (50, 33) is off a twenty-row window: not drawn, not in the legend,
    // still in the table. The spots go down FIRST (16b cut V.1): A, then B,
    // which X3 then covers; C at (50, 25) is off the window.
    expect(win.markers.map((m) => m.designator)).toEqual([
      "The Ford",
      "The Roof",
      "RA",
      "BA",
      "RB",
      "X3",
    ]);
    expect(cockpitView(theFrame(), win, []).legend).toBe("x BA  + RB  ? X3  A The Ford");
  });

  it("names the spots after the contacts, and only those that own their cell (16b cut V.1)", () => {
    const frame = theFrame();
    // The default 128-cell view holds the whole 96 by 64 ground: X4 and C are
    // inside it, B is still under X3.
    const whole = composeWindow(theMap(), frame, 128, 128);
    expect(markerLegend(whole)).toBe("x BA  + RB  ? X3  ? X4  A The Ford  C The Woods");
    // A spot under a contact is not listed; move X3 off it and it is.
    const moved: CockpitFrame = {
      ...frame,
      contacts: frame.contacts.map((c) => (c.designator === "X3" ? { ...c, cell: [15, 7] } : c)),
    };
    expect(markerLegend(composeWindow(theMap(), moved, 39, 20))).toBe(
      "x BA  + RB  ? X3  A The Ford  B The Roof",
    );
    // A spot under the pilot's own `@` is the `@`'s, and the `@` is never listed.
    const home: ArenaMap = {
      ...theMap(),
      spots: [{ letter: "A", name: "Home Plate", cell: [6, 4] }],
    };
    const under = composeWindow(home, frame, 39, 20);
    expect(under.grid[under.h - 1 - 4]?.charAt(6)).toBe("@");
    expect(markerLegend(under)).toBe("x BA  + RB  ? X3");
  });

  it("names in the legend only the marker that owns its cell (16b cut B.3, B-62)", () => {
    // Two contacts on one cell: the later is drawn, so the later is named —
    // the TUI's `render_cockpit_map_rows` rule. A contact on the pilot's own
    // cell overwrites the `@` (drawn first) and is named; the `@` never is.
    const frame = theFrame();
    const [ba, rb, x3] = frame.contacts;
    const stacked: CockpitFrame = {
      ...frame,
      contacts: [
        { ...ba!, cell: [2, 9] },
        { ...x3!, cell: [14, 7] },
        { ...rb!, cell: [2, 9] },
        { ...x3!, designator: "X5", cell: [6, 4] },
      ],
    };
    const win = composeWindow(theMap(), stacked, 78, 20);
    // 16b cut V.1: the two spots inside 78 by 20 go down first; B is under X3.
    expect(win.markers.map((m) => m.designator)).toEqual([
      "The Ford",
      "The Roof",
      "RA",
      "BA",
      "X3",
      "RB",
      "X5",
    ]);
    expect(win.grid[win.h - 1 - 9]?.charAt(2)).toBe("+");
    expect(win.grid[win.h - 1 - 4]?.charAt(6)).toBe("?");
    expect(cockpitView(stacked, win, []).legend).toBe("? X3  + RB  ? X5  A The Ford");
  });

  it("subtracts a NON-ZERO window origin from every absolute cell", () => {
    // The committed fixture cannot catch this: its own origin is [0, 0].
    const frame: CockpitFrame = {
      ...theFrame(),
      cell: [60, 40],
      contacts: [
        { ...theFrame().contacts[0]!, designator: "BA", cell: [55, 38] },
        { ...theFrame().contacts[2]!, designator: "X9", cell: [6, 4] },
      ],
    };
    const win = composeWindow(theMap(), frame, 20, 10);
    expect(win).toMatchObject({ x0: 50, y0: 35, w: 20, h: 10 });
    const at = (x: number, y: number): string => win.grid[win.h - 1 - y]?.charAt(x) ?? "";
    expect(at(10, 5)).toBe("@"); // 60,40 → 10,5
    expect(at(5, 3)).toBe("x"); // 55,38 → 5,3
    expect(win.markers.map((m) => m.designator)).toEqual(["RA", "BA"]);
  });

  it("clamps at every edge and centres in the middle", () => {
    const map = theMap(); // 96 by 64
    const base = theFrame();
    const w = (cell: [number, number], cols = 128, rows = 128) =>
      composeWindow(map, { ...base, cell }, cols, rows);
    // The default viewport is bigger than this arena in both axes: the whole
    // arena, pinned at its corner.
    expect(w([6, 4])).toMatchObject({ x0: 0, y0: 0, w: 96, h: 64 });
    // A 20 by 10 window: centred, then clamped at each of the four edges.
    expect(w([48, 32], 20, 10)).toMatchObject({ x0: 38, y0: 27, w: 20, h: 10 });
    expect(w([0, 0], 20, 10)).toMatchObject({ x0: 0, y0: 0 });
    expect(w([95, 63], 20, 10)).toMatchObject({ x0: 76, y0: 54 });
    // Zoomed in and out on the same cell, both still clamped to the arena.
    expect(w([48, 32], 32, 32)).toMatchObject({ x0: 32, y0: 16, w: 32, h: 32 });
    expect(w([48, 32], 256, 256)).toMatchObject({ x0: 0, y0: 0, w: 96, h: 64 });
    // No cell at all: the arena's middle.
    expect(composeWindow(map, { ...base, cell: null }, 20, 10)).toMatchObject({
      x0: 38,
      y0: 27,
    });
  });

  it("answers exactly h rows of w glyphs even on a map with nothing in it", () => {
    const empty: ArenaMap = {
      seated: true,
      match: 1,
      surface: "",
      cell_cm: 1000,
      width: 4,
      height: 3,
      origin_cell: [0, 0],
      rows: [],
      legend: [],
      spots: [],
    };
    const win = composeWindow(empty, { ...theFrame(), cell: [1, 1], contacts: [] }, 10, 10);
    expect(win.grid).toEqual(["    ", " @  ", "    "]);
  });
});

/** One notice, with every key this build reads defaulted — so a case says only
 * what it is about. */
function notice(partial: Partial<CockpitNotice> & { kind: string }): string {
  return noticeLine({
    designator: null,
    was: null,
    section: null,
    cause: null,
    refusal: null,
    order: null,
    weapon: null,
    detail: null,
    friendly: false,
    spent: false,
    ...partial,
  });
}

describe("the view model", () => {
  it("prints the status block, the lock line, damage, weapons and contacts", () => {
    const frame = theFrame();
    expect(statusLines(frame)).toEqual([
      "Callsign RA",
      "power online",
      "43 kph  heading 045°",
      "heat 40/100",
      // 16b cut D.2 (16c `T-59`): this platform's own scope, not the arena's
      // one global cadence word.
      "scope hostile",
      // 16b cut D.4 (16c `T-20`): the lock reads to what the plot's own age
      // can carry — 1,200 ms is 20 m of drift, so a 20 m step, and 430 rounds
      // half up to 440.
      "Lock BA  440 m  62%",
    ]);
    expect(weaponLines(frame)).toEqual([
      "rules: S1 fire; MIND RULES",
      "cannon  ready  14/20  62%",
      "machine_gun  1 s  380/500  21%",
    ]);
    const view = cockpitView(frame, null, []);
    expect(view.damage.map(damageText)[3]).toBe("right_track  destroyed  0/24  right 0/24");
    // The bars are arithmetic on the payload's own two numbers and say nothing
    // the fraction beside them does not.
    expect(view.damage.map((d) => d.pct)).toEqual([100, 100, 25, 0, 100]);
    expect(view.damage[1]?.facets.map((f) => f.pct)).toEqual([69, 100]);
    expect(view.damage[0]?.facets).toEqual([]);
    // 16b cut D.1: the band is the PILOT's word now — thirds of the lesser of
    // the platform's own 600 m sight and the cannon's reach — so the scene's
    // 450 m lock reads `far`. 17b cut M1a.3: then the aspect's side word and
    // the main gun's odds.
    expect(view.contacts.map(contactText)[0]).toBe(
      "BA  blue  045°  far  450 m  1 s  side  cannon 62%",
    );
    // The off-window contact is in the table even though no marker drew.
    expect(view.contacts.map((c) => c.designator)).toContain("X4");
    expect(scanText(scanChips(frame.contacts[0]!, frame.scan_tables))).toBe(
      "  engine hull left_track/y right_track/r turret/d ",
    );
    expect(scanChips(frame.contacts[1]!, frame.scan_tables)).toEqual([]);
  });

  it("says a held platform, an overheated one, an empty rack and no lock", () => {
    const frame = theFrame();
    expect(
      statusLines({
        ...frame,
        blocked: true,
        heat: { heat: 101, max: 100, overheated: true },
        lock_designator: null,
        lock_range_m: null,
        lock_percent: null,
      }),
    ).toEqual([
      "Callsign RA",
      "power online  HELD",
      "43 kph  heading 045°",
      "heat 101/100  OVERHEATED",
      "scope hostile",
      "No lock",
    ]);
    expect(statusLines({ ...frame, heat: null })).not.toContain("heat 40/100");
    expect(weaponLines({ ...frame, weapons: [], rules: [] })).toEqual(["(no weapon)"]);
    expect(cockpitViewLines(cockpitView({ ...frame, contacts: [] }, null, []))).toContain(
      "(no contacts)",
    );
  });

  it("composes a notice from kind + designator + section, never from text", () => {
    expect(noticeLines(theFrame().notices)).toEqual([
      "X4 new contact",
      "BA section destroyed: turret",
    ]);
    expect(notice({ kind: "power_online" })).toBe("power online");
    expect(notice({ kind: "hit", designator: "BA", section: "hull" })).toBe("BA hit: hull");
    expect(notice({ kind: "struck" })).toBe("hit taken");
    // A kind this build predates reads as its own wire word, which can name
    // nothing the payload did not already carry.
    expect(notice({ kind: "some_new_thing", designator: "RB" })).toBe("RB some new thing");
  });

  // **16b cut D.2** (`B-07`): until this cut a refusal and a block carried their
  // whole sentence in `text`, which this ring correctly never reads — so the
  // panel printed a bare `refused` and a bare `held`. Every word below is
  // composed from the notice's own fields.
  it("says WHICH order was refused, WHY, and with what numbers", () => {
    expect(notice({ kind: "refused", order: "heading", refusal: "power_not_online" })).toBe(
      "HEADING refused: power is not online",
    );
    expect(notice({ kind: "refused", order: "heading", refusal: "queue_full" })).toBe(
      "HEADING refused: too many orders at once",
    );
    expect(notice({ kind: "refused", order: "heading", refusal: "wreck" })).toBe(
      "HEADING refused: the tank is destroyed",
    );
    expect(notice({ kind: "refused", order: "speed", refusal: "motion_disabled" })).toBe(
      "SPEED refused: the tank cannot move",
    );
    expect(notice({ kind: "refused", order: "lock", refusal: "no_contact" })).toBe(
      "LOCK refused: no such contact",
    );
    expect(notice({ kind: "refused", order: "fire", weapon: "cannon", refusal: "no_lock" })).toBe(
      "FIRE cannon refused: no lock",
    );
    expect(
      notice({
        kind: "refused",
        order: "fire",
        weapon: "cannon",
        refusal: "cycling",
        detail: { secs: 3, heat: null, max: null },
      }),
    ).toBe("FIRE cannon refused: still cycling, 3 s");
    expect(notice({ kind: "refused", order: "fire", weapon: "cannon", refusal: "empty" })).toBe(
      "FIRE cannon refused: empty",
    );
    expect(
      notice({
        kind: "refused",
        order: "fire",
        weapon: "laser",
        refusal: "overheated",
        detail: { secs: null, heat: 60, max: 60 },
      }),
    ).toBe("FIRE laser refused: overheated, 60 of 60");
    expect(notice({ kind: "refused", order: "fire", weapon: "ghost", refusal: "no_weapon" })).toBe(
      "FIRE ghost refused: no such weapon",
    );
    expect(notice({ kind: "refused", order: "startup", refusal: "epoch" })).toBe(
      "STARTUP refused: the seat changed hands",
    );
    // A refusal word this build predates opens its own underscores; a refusal
    // with no word at all still names the order.
    expect(notice({ kind: "refused", order: "fire", refusal: "brand_new" })).toBe(
      "FIRE refused: brand new",
    );
    expect(notice({ kind: "refused", order: "fire" })).toBe("FIRE refused: refused");
  });

  it("says WHY the tank is held", () => {
    expect(notice({ kind: "blocked", cause: "impassable" })).toBe(
      "held: the ground will not take the tank",
    );
    expect(notice({ kind: "blocked", cause: "occupied" })).toBe("held: something is in the way");
    expect(notice({ kind: "blocked", cause: "edge" })).toBe("held: the arena's edge");
    expect(notice({ kind: "blocked", cause: "too_steep" })).toBe("held: too steep: drive round");
    expect(notice({ kind: "blocked", cause: "sand_trap" })).toBe("held: sand trap");
  });

  it("marks a friendly, a spent section, and the pilot's own two lines", () => {
    // 16b cut D.2 (`B-18`): said on the panel exactly as in the scrollback.
    expect(notice({ kind: "locked", designator: "RB", friendly: true })).toBe(
      "RB (FRIENDLY) locked",
    );
    expect(notice({ kind: "hit", designator: "RB", section: "turret", friendly: true })).toBe(
      "RB (FRIENDLY) hit: turret",
    );
    expect(notice({ kind: "hit", designator: "BA", section: "turret", friendly: false })).toBe(
      "BA hit: turret",
    );
    // 16b cut D.2 (`B-15`).
    expect(notice({ kind: "hit", designator: "BA", section: "turret", spent: true })).toBe(
      "BA hit: turret (already destroyed)",
    );
    // 16b cut B.1 (`T-86`): `FrameNotice::section()` answers the victim's own
    // section on `struck_spent` (and `struck`, `section_lost`) now, so the
    // panel names it through the common clause; a notice with none still
    // reads the bare words.
    expect(notice({ kind: "struck_spent", section: "left_track", spent: true })).toBe(
      "hit on a dead section: left_track",
    );
    expect(notice({ kind: "struck_spent" })).toBe("hit on a dead section");
    // 16b cut D.2 (`B-03`): the pilot's own tank, naming nothing else.
    expect(notice({ kind: "destroyed" })).toBe("your tank is destroyed");
    expect(notice({ kind: "disabled" })).toBe("your tank is out of the fight");
  });

  // The golden: the whole of the screen's text for the committed frame. It is
  // what the player reads, because the component renders these same strings
  // and decides none of them itself.
  it("matches the committed view-model golden", () => {
    const frame = theFrame();
    const win = composeWindow(theMap(), frame, 78, 20);
    const lines = cockpitViewLines(cockpitView(frame, win, noticeLines(frame.notices)));
    const path = resolve(process.cwd(), DIR, "web_view_model.txt");
    const text = `${lines.join("\n")}\n`;
    if (process.env.COCKPIT_BLESS === "1") {
      writeFileSync(path, text, "utf8");
    }
    expect(text).toBe(readFileSync(path, "utf8").replace(/\r\n/g, "\n"));
  });
});

// **16b cut B.1**: the web composer's cut, the pure half — the forced labels
// (`B-09`), the unknown band (`T-58`), the two panel rows and the ring's new
// shapes (`T-86`), and the parity fixture both rings are held to.
describe("16b cut B.1", () => {
  it("decodes `was` on a notice and the frame's `receiver`", () => {
    const raw = json("cockpit_frame.json") as Record<string, unknown>;
    // The committed scene: a clean receiver, and no notice quoting an old name.
    const frame = theFrame();
    expect(frame.receiver).toBeNull();
    expect(frame.notices.map((n) => n.was)).toEqual([null, null]);
    const jammed = decodeCockpitFrame({
      ...raw,
      receiver: { budget: 15, budget_max: 40, reach_percent: 60 },
      notices: [{ kind: "unidentified", designator: "X2", was: "BA\u0007LONGERNAME" }],
    });
    expect(jammed?.receiver).toEqual({ budget: 15, budget_max: 40, reach_percent: 60 });
    // Cleaned and cut to a designator's eight, like `designator`.
    expect(jammed?.notices[0]?.was).toBe("BALONGER");
    // A server that predates the key sends neither.
    const older = { ...raw };
    delete older.receiver;
    expect(decodeCockpitFrame(older)?.receiver).toBeNull();
    expect(decodeCockpitFrame({ ...raw, receiver: "degraded" })?.receiver).toBeNull();
  });

  it("prints the receiver row after heat, and the tank's own limit on the speed line", () => {
    const frame = theFrame();
    const receiver = { budget: 15, budget_max: 40, reach_percent: 60 };
    expect(statusLines({ ...frame, receiver })).toEqual([
      "Callsign RA",
      "power online",
      "43 kph  heading 045°",
      "heat 40/100",
      "receiver 15/40  60%",
      "scope hostile",
      "Lock BA  440 m  62%",
    ]);
    // 19 characters: never wraps the TUI's 22-column pane.
    expect("receiver 15/40  60%").toHaveLength(19);
    expect(statusLines({ ...frame, wreck: true, immobile: true })[2]).toBe(
      "43 kph  heading 045°  DESTROYED",
    );
    expect(statusLines({ ...frame, immobile: true })[2]).toBe("43 kph  heading 045°  IMMOBILE");
    // 16c Q-PT1: a range says so on the callsign row, as the TUI's pane does.
    expect(statusLines({ ...frame, mode: "range" })[0]).toBe("Callsign RA  RANGE");
    expect(frame.mode).toBe("skirmish");
    // The committed frame prints none of them.
    for (const line of statusLines(frame)) {
      expect(line).not.toMatch(/receiver|DESTROYED|IMMOBILE|RANGE/);
    }
  });

  it("leaves an unknown band letter uncoloured", () => {
    const frame = theFrame();
    const contact = { ...frame.contacts[0]!, condition: { table: 0, bands: "gz", age_ms: 0 } };
    expect(scanChips(contact, frame.scan_tables).map((c) => c.band)).toEqual([
      "green",
      "unknown",
    ]);
  });

  it("labels a hostile and the lock at every zoom, the rest from 6 px", () => {
    expect(labelledAt("hostile", "BA", null, 2)).toBe(true);
    expect(labelledAt("unknown", "X3", null, 4)).toBe(false);
    expect(labelledAt("unknown", "X3", null, 6)).toBe(true);
    expect(labelledAt("unknown", "X3", "X3", 2)).toBe(true); // the lock
    expect(labelledAt("friendly", "RB", null, 5)).toBe(false);
    expect(labelledAt("you", "RA", null, 5)).toBe(false);
    expect(labelledAt("you", "RA", null, 8)).toBe(true);
    // 16b cut V.1: a named spot keeps the friendly's rule — from 6 px — and is
    // never forced by a name that happens to read like the lock.
    expect(labelledAt("spot", "The Ford", null, 5)).toBe(false);
    expect(labelledAt("spot", "The Ford", null, 6)).toBe(true);
    expect(labelledAt("spot", "BA", "BA", 2)).toBe(false);
  });

  it("labels the spots after every contact, and never a covered one (16b cut V.1)", () => {
    // The committed scene at 64 cells (8 px): X3 stands on The Roof. The
    // contact's label is placed first and the spot's box would overlap it — and
    // a spot under a contact is drawn nowhere, so it is labelled nowhere.
    const frame = theFrame();
    const win = composeWindow(theMap(), frame, 64, 64);
    const flags = labelPlan(win.markers, frame.lock_designator, 8);
    const labelled = win.markers.filter((_, i) => flags[i]).map((m) => m.designator);
    expect(labelled).toContain("X3");
    expect(labelled).not.toContain("The Roof");
    // Every other marker is labelled; the list is in MARKER order (all three
    // spots were drawn first), which is not the order the labels were placed in.
    expect(labelled).toEqual(["The Ford", "The Woods", "RA", "BA", "RB", "X3", "X4"]);
    // A spot one cell west of a contact, drawn FIRST: its label box runs over
    // the contact's, and the contact's is placed first anyway — the spot gives
    // way even though it came earlier in the drawing.
    const m = (x: number, designator: string, kind: Marker["kind"]): Marker => ({
      x,
      y: 10,
      glyph: kind === "spot" ? "A" : "?",
      kind,
      designator,
    });
    const pair = [m(40, "The Long Field", "spot"), m(41, "X3", "unknown")];
    expect(labelPlan(pair, null, 8)).toEqual([false, true]);
    // With room, both.
    const apart = [m(10, "The Long Field", "spot"), m(41, "X3", "unknown")];
    expect(labelPlan(apart, null, 8)).toEqual([true, true]);
  });

  it("skips an overlapping label in frame order, and never the lock's", () => {
    const m = (x: number, designator: string, kind: Marker["kind"] = "hostile"): Marker => ({
      x,
      y: 10,
      glyph: "x",
      kind,
      designator,
    });
    const two = [m(40, "BA"), m(42, "BB")];
    // Two cells apart at 2 px the second name would sit on the first.
    expect(labelPlan(two, null, 2)).toEqual([true, false]);
    // The lock is placed first, so it is the other one that gives way.
    expect(labelPlan(two, "BB", 2)).toEqual([false, true]);
    // At 8 px they are sixteen pixels apart and both fit.
    expect(labelPlan(two, null, 8)).toEqual([true, true]);
    // At 6 px the font is still 8 px (`max(8, px)`), so three glyphs are 14.4 px
    // wide and the twelve pixels to the next marker do not hold them: measured
    // at `px * 0.6` the two would both be painted, one over the other.
    const three = [m(40, "X10"), m(42, "X11")];
    expect(labelPlan(three, null, 6)).toEqual([true, false]);
    expect(labelPlan(three, null, 8)).toEqual([true, true]);
    // An unknown under 6 px is not labelled at all, and costs no room.
    expect(labelPlan([m(40, "X3", "unknown"), m(42, "BB")], null, 2)).toEqual([false, true]);
    // The committed scene at the default 128-cell view (the whole 96-cell
    // ground at 5 px): the hostile lock is labelled, the pilot's own marker,
    // the friendly and the two unknowns are not.
    const frame = theFrame();
    const win = composeWindow(theMap(), frame, 128, 128);
    const flags = labelPlan(win.markers, frame.lock_designator, 5);
    expect(win.markers.filter((_, i) => flags[i]).map((mk) => mk.designator)).toEqual(["BA"]);
  });

  it("words every notice in `notice_lines.json` as the file says", () => {
    const pairs = json("notice_lines.json") as { notice: unknown; line: string }[];
    expect(pairs.length).toBeGreaterThanOrEqual(12);
    const kinds = new Set<string>();
    for (const { notice: raw, line } of pairs) {
      // Through the REAL decoder: the notice rides a frame, as on the wire.
      const frame = decodeCockpitFrame({
        ...(json("cockpit_frame.json") as Record<string, unknown>),
        notices: [raw],
      });
      const decoded = frame?.notices[0];
      expect(decoded, JSON.stringify(raw)).toBeDefined();
      expect(noticeLine(decoded!), JSON.stringify(raw)).toBe(line);
      kinds.add(decoded!.kind);
    }
    // Every kind on the committed frame is covered, and so is every case the
    // file exists for.
    for (const kind of theFrame().notices.map((n) => n.kind)) {
      expect(kinds).toContain(kind);
    }
    for (const kind of [
      "refused",
      "blocked",
      "unidentified",
      "classified",
      "hit",
      "locked",
      "struck",
      "struck_spent",
      "section_lost",
      "constructor",
    ]) {
      expect(kinds).toContain(kind);
    }
    // An own-side tank told apart keeps its mark on the name it reads now, so
    // it never reads as a hostile's identification.
    expect(pairs.map((p) => p.line)).toContain("X3 identified: RB (FRIENDLY)");
    // 17b cut M1b.4: the five slot cases.
    for (const kind of ["slot_fired", "slot_held"]) {
      expect(kinds).toContain(kind);
    }
    expect(pairs.map((p) => p.line)).toContain(
      "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less",
    );
  });

  it("17b cut M1b.4: words a slot's shot and every held why as the line-mode sentence", () => {
    const decode = (raw: Record<string, unknown>) => {
      const frame = decodeCockpitFrame({
        ...(json("cockpit_frame.json") as Record<string, unknown>),
        notices: [raw],
      });
      const decoded = frame?.notices[0];
      expect(decoded, JSON.stringify(raw)).toBeDefined();
      return noticeLine(decoded!);
    };
    const held = (detail: Record<string, unknown>) =>
      decode({
        kind: "slot_held",
        slot: 1,
        weapon: "cannon",
        designator: "BA",
        friendly: false,
        detail,
      });
    const range = (min: number, max: number) =>
      held({
        why: "range",
        percent: null,
        floor: null,
        aspect: null,
        range_m: 650,
        min_m: min,
        max_m: max,
        rounds: null,
        keeps: null,
      });
    expect(range(200, 600)).toBe("S1 held the cannon: BA at 650 m, outside 200 to 600");
    expect(range(0, 600)).toBe("S1 held the cannon: BA at 650 m, outside under 600");
    expect(range(200, 4294967295)).toBe("S1 held the cannon: BA at 650 m, outside past 200");
    expect(
      held({
        why: "rounds",
        percent: null,
        floor: null,
        aspect: null,
        range_m: null,
        min_m: null,
        max_m: null,
        rounds: 7,
        keeps: 8,
      }),
    ).toBe("S1 held the cannon: 7 rounds, keeps 8");
    expect(
      decode({
        kind: "slot_fired",
        slot: 2,
        weapon: "machine_gun",
        designator: "RB",
        friendly: true,
        percent: 95,
        aspect: "right",
        detail: null,
      }),
    ).toBe("Machine_gun fires at RB (FRIENDLY) by S2 (95%, side)");
    // A part missing: the kind's own words, as the TUI's ring.
    expect(decode({ kind: "slot_fired", weapon: "cannon" })).toBe("fires by");
    expect(decode({ kind: "slot_held", slot: 1, weapon: "cannon" })).toBe("held");
    expect(
      decode({
        kind: "refused",
        order: "fire",
        weapon: "cannon",
        refusal: "unlawful",
        detail: null,
      }),
    ).toBe("FIRE cannon refused: that rule is not lawful for this unit");
    expect(
      decode({
        kind: "refused",
        order: "fire",
        weapon: "cannon",
        refusal: "unlawful",
        detail: { floor_permille: 100 },
      }),
    ).toBe("FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less");
    // Every other notice decodes to exactly the keys it always had.
    const plain = decodeCockpitFrame({
      ...(json("cockpit_frame.json") as Record<string, unknown>),
      notices: [{ kind: "refused", order: "fire", refusal: "cycling", detail: { secs: 3 } }],
    })?.notices[0];
    expect(plain && "slot" in plain).toBe(false);
    expect(plain?.detail).toEqual({ secs: 3, heat: null, max: null });
  });
});

describe("16b cut Y.4: the catch-up report", () => {
  type Case = { callsign: string; catchup: unknown; lines: string[] };

  it("decodes every case of `catchup_lines.json` and words it as the file says", () => {
    const cases = json("catchup_lines.json") as Case[];
    // Nine until 17b cut M5.3, which added the tenth (D14).
    expect(cases).toHaveLength(10);
    for (const { callsign, catchup, lines } of cases) {
      const decoded = decodeCatchup(catchup);
      expect(decoded, JSON.stringify(catchup)).not.toBeNull();
      expect(catchupLines(callsign, decoded!)).toEqual(lines);
      // And through the real frame decoder, as it rides the wire.
      const frame = decodeCockpitFrame({
        ...(json("cockpit_frame.json") as Record<string, unknown>),
        callsign,
        catchup,
      });
      expect(frame?.catchup).toEqual(decoded);
    }
    // The full case decodes to the fixture's own figures.
    const full = decodeCatchup(cases[0]!.catchup);
    expect(full?.away_s).toBe(134);
    expect(full?.cell_then).toEqual([64, 97]);
    expect(full?.contacts_new).toEqual(["BA", "X3"]);
    expect(full?.sections.map((s) => s.id)).toEqual(["left_track", "hull"]);
    expect(full?.machine).toEqual({ state: "withdraw", goal: [2, 97], objective: null });
  });

  it("decodes a malformed report to null, drops a malformed entry and bounds every string", () => {
    for (const bad of ["Back aboard", 7, null, [], undefined]) {
      expect(decodeCatchup(bad)).toBeNull();
    }
    // The TUI's `catchup_words_refuse_a_malformed_report_and_bound_every_field`,
    // case for case.
    const hostile = decodeCatchup({
      away_s: 61,
      cell_then: [1, -2],
      cell_now: ["x", 3],
      power_now: "on\u001b[31mline",
      wreck: "yes",
      contacts_new: ["ABCDEFGHIJ", 3, "  "],
      contacts_lost: [],
      contacts_more: 0,
      sections: [
        {
          id: "s".repeat(30),
          then: { condition: "intact", armour: 9 },
          now: { condition: "intact", armour: 4 },
        },
        "junk",
      ],
      rounds_spent: [{ weapon: "w\u0007", rounds: 2 }, { weapon: "x" }],
      machine: { state: "a_new_state_word_x", objective: null },
    });
    expect(hostile).not.toBeNull();
    expect(catchupLines("RA", hostile!)).toEqual([
      "Back aboard RA after 1 m 1 s away.",
      "  The machine was a new state word under no order.",
      "  Position: unknown.",
      "  Contacts: new ABCDEFGH; none lost.",
      `  Sections: ${"s".repeat(24)} armour 9 then, 4 now.`,
      "  Rounds spent: w 2.",
      "  Power on[31mline.",
    ]);
    // Lists are cut to what the arena sends at most.
    const long = decodeCatchup({
      contacts_new: Array.from({ length: 20 }, (_, n) => `X${n + 1}`),
      sections: Array.from({ length: 9 }, (_, n) => ({
        id: `s${n}`,
        then: { condition: "intact", armour: 2, internals: 1 },
        now: { condition: "intact", armour: 1, internals: 1 },
      })),
      rounds_spent: Array.from({ length: 9 }, (_, n) => ({ weapon: `w${n}`, rounds: 1 })),
    });
    expect(long?.contacts_new).toHaveLength(12);
    expect(long?.sections).toHaveLength(6);
    expect(long?.rounds_spent).toHaveLength(6);
  });
});

// **17b cut M5.3 (D14, doc 17 §24's index)**: the machine's `unknown` — a hull
// flown by a mind that is not Standard Doctrine — said as the mind's own, the
// tenth case of `catchup_lines.json`, in parity with the server's
// `machine_state_words` and the TUI's `catchup_words` (each a test of this name).
describe("17b cut M5.3", () => {
  type Case = { callsign: string; catchup: unknown; lines: string[] };
  const D14 = "  The machine was flying on its own mind under no order.";

  it("m5_d14_the_catch_up_says_its_own_mind_in_three_renderers", () => {
    const cases = json("catchup_lines.json") as Case[];
    const said = cases.filter(
      (c) => (c.catchup as { machine: { state: string } | null }).machine?.state === "unknown",
    );
    expect(said).toHaveLength(1);
    const decoded = decodeCatchup(said[0]!.catchup);
    expect(decoded?.machine?.state).toBe("unknown");
    const lines = catchupLines(said[0]!.callsign, decoded!);
    expect(lines[1]).toBe(D14);
    expect(lines).toEqual(said[0]!.lines);
    expect(lines.some((line) => line.includes("unknown under"))).toBe(false);
    // Under each order a machine can hold, the same phrase, under 120.
    for (const objective of ["hold", "advance", "engage", "withdraw", "screen", "support"]) {
      const machine = { state: "unknown", goal: null, objective };
      const line = catchupLines("RA", { ...decoded!, machine })[1]!;
      expect(line.startsWith("  The machine was flying on its own mind under a")).toBe(true);
      expect(line.length).toBeLessThan(120);
    }
    // The six doctrine states keep their words.
    for (const state of ["startup", "patrol", "approach", "engage", "withdraw", "disabled"]) {
      const machine = { state, goal: null, objective: null };
      const line = catchupLines("RA", { ...decoded!, machine })[1]!;
      expect(line).not.toContain("mind");
    }
  });
});

// **16b cut O.2** (16c N12): the pod's switch — decoded off the frame's `ecm`,
// printed on the scope row as the TUI prints it, and its two notices and the
// refusal worded short, in parity.
describe("16b cut O.2", () => {
  it("decodes `ecm`, and reads an absent or unknown key as null", () => {
    const raw = json("cockpit_frame.json") as Record<string, unknown>;
    // The committed scene carries no pod.
    expect(raw.ecm).toBeNull();
    expect(theFrame().ecm).toBeNull();
    expect(decodeCockpitFrame({ ...raw, ecm: "on" })?.ecm).toBe("on");
    expect(decodeCockpitFrame({ ...raw, ecm: "off" })?.ecm).toBe("off");
    const older = { ...raw };
    delete older.ecm;
    expect(decodeCockpitFrame(older)?.ecm).toBeNull();
    expect(decodeCockpitFrame({ ...raw, ecm: "sideways" })?.ecm).toBeNull();
    expect(decodeCockpitFrame({ ...raw, ecm: true })?.ecm).toBeNull();
  });

  it("carries the switch on the scope row and adds no row", () => {
    const frame = theFrame();
    const plain = statusLines(frame);
    expect(plain).toContain("scope hostile");
    const on = statusLines({ ...frame, ecm: "on" });
    expect(on).toHaveLength(plain.length);
    expect(on).toContain("scope hostile  EMCON on");
    const off = statusLines({ ...frame, ecm: "off" });
    expect(off).toContain("scope hostile  EMCON off");
    expect(statusLines({ ...frame, ecm: null })).toEqual(plain);
    // The longest scope word with the longer suffix fits the TUI's 32 columns.
    const longest = statusLines({ ...frame, ecm: "off", scope: "unidentified" });
    expect(longest).toContain("scope unidentified  EMCON off");
    for (const line of longest) {
      expect(line.length).toBeLessThanOrEqual(32);
    }
  });

  it("words the two switch notices and the refusal short", () => {
    expect(notice({ kind: "emcon_on" })).toBe("EMCON on");
    expect(notice({ kind: "emcon_off" })).toBe("EMCON off: pod silent");
    expect(notice({ kind: "refused", order: "emcon", refusal: "no_jammer" })).toBe(
      "EMCON refused: no jamming pod",
    );
  });
});

// **17b cut M1a.3** (doc 17 §3, D-UM-4, D-UM-5): the fields a mind reads joined
// the frame at `FRAME_VERSION` 2, and the contact row grew the aspect and the
// main gun's odds. Every new key is optional to this decoder.
describe("17b cut M1a.3", () => {
  it("decodes the new keys from the committed frame", () => {
    const frame = theFrame();
    expect(frame.armed).toBe(true);
    expect(frame.t_ms).toBe(61250);
    expect(frame.order).toBeNull();
    expect(frame.contacts.map((c) => c.aspect)).toEqual(["left", "front", "front", "rear"]);
    expect(frame.contacts.map((c) => c.reads_dead)).toEqual([false, false, false, false]);
    expect(frame.contacts[0]?.odds).toEqual([
      { weapon: "cannon", p_permille: 624, percent: 62 },
      { weapon: "machine_gun", p_permille: 210, percent: 21 },
    ]);
    expect(frame.contacts[2]?.odds).toEqual([]);
    expect(frame.contacts[0]?.fatal).toEqual(["hull"]);
    expect(frame.contacts[1]?.fatal).toBeNull();
    expect(frame.weapons.map((w) => w.bands_cm)).toEqual([[100000, 300000], [40000]]);
  });

  it("decodes an older frame, with none of the keys, to the defaults", () => {
    const old = decodeCockpitFrame({
      match: 3,
      callsign: "BB",
      contacts: [{ designator: "X1", class: "unknown" }],
      weapons: [{ id: "gun" }],
    });
    expect(old?.t_ms).toBeNull();
    expect(old?.order).toBeNull();
    expect(old?.armed).toBe(true);
    expect(old?.contacts[0]?.aspect).toBe("front");
    expect(old?.contacts[0]?.reads_dead).toBeNull();
    expect(old?.contacts[0]?.odds).toEqual([]);
    expect(old?.contacts[0]?.fatal).toBeNull();
    expect(old?.weapons[0]?.bands_cm).toEqual([]);
  });

  it("decodes an unarmed frame, whose rows end at the side word", () => {
    const raw = json("cockpit_frame.json") as Record<string, unknown>;
    const unarmed: Record<string, unknown> = {
      ...raw,
      contacts: (raw.contacts as Record<string, unknown>[]).map((c) => {
        const { odds: _odds, reads_dead: _dead, ...rest } = c;
        return rest;
      }),
    };
    delete unarmed.weapons;
    unarmed.rules = [];
    const frame = decodeCockpitFrame(unarmed);
    expect(frame?.armed).toBe(false);
    expect(frame?.weapons).toEqual([]);
    expect(frame?.contacts.map((c) => c.reads_dead)).toEqual([null, null, null, null]);
    expect(frame ? weaponLines(frame) : []).toEqual(["(no weapon)"]);
    expect(frame ? contactRows(frame).map(contactText) : []).toEqual([
      "BA  blue  045°  far  450 m  1 s  side",
      "RB  red  270°  close  100 m  0 s  front",
      "X3  unknown  315°  effective  200 m  3 s  front",
      "X4  unknown  030°  far  600 m  6 s  rear",
    ]);
  });

  it("pins the armed rows: the main gun's odds, or a dash", () => {
    expect(contactRows(theFrame()).map((row) => row.detail)).toEqual([
      "blue  045°  far  450 m  1 s  side  cannon 62%",
      // 17b cut M1b.3 (Q2): RB is red's own, and the rows never carry a
      // friend's odds — the dash.
      "red  270°  close  100 m  0 s  front  -",
      "unknown  315°  effective  200 m  3 s  front  -",
      "unknown  030°  far  600 m  6 s  rear  -",
    ]);
    expect(["front", "left", "right", "rear", "top"].map(sideWord)).toEqual([
      "front",
      "side",
      "side",
      "rear",
      "front",
    ]);
  });

  it("decodes an order with its payloads and designators", () => {
    const raw = json("cockpit_frame.json") as Record<string, unknown>;
    const frame = decodeCockpitFrame({
      ...raw,
      order: {
        squad: "red-1",
        objective: "engage",
        target: "BA",
        to: null,
        facing_mdeg: null,
        of: null,
        hold_fire: false,
        max_range_band: "effective",
        expires_in_ms: 54250,
      },
    });
    expect(frame?.order).toEqual({
      squad: "red-1",
      objective: "engage",
      target: "BA",
      to: null,
      facing_mdeg: null,
      of: null,
      hold_fire: false,
      max_range_band: "effective",
      expires_in_ms: 54250,
    });
    expect(decodeCockpitFrame({ ...raw, order: "hold" })?.order).toBeNull();
  });
});

// **17b cut M1b.3** (doc 17 §3, §5, D-UM-4, D-UM-17): `FRAME_VERSION` 3 — each
// weapon's `armed` slot and the match's `odds_floor_permille` — and the arming
// line under an armed weapon, the arena's one formatter written out here.
describe("17b cut M1b.3", () => {
  const arming = (partial: Partial<CockpitArming>): CockpitArming => ({
    slot: 1,
    target: "lock",
    min_permille: 600,
    percent: 60,
    aspects: ["front", "left", "right", "rear"],
    keeps: 0,
    min_range_m: null,
    max_range_m: null,
    ...partial,
  });

  it("decodes the armed row, the null row and the floor from the committed frame", () => {
    const frame = theFrame();
    expect(frame.odds_floor_permille).toBe(100);
    expect(frame.weapons[0]?.armed).toEqual({
      slot: 1,
      target: "lock",
      min_permille: 600,
      percent: 60,
      aspects: ["left", "right", "rear"],
      keeps: 8,
      min_range_m: null,
      max_range_m: null,
    });
    expect(frame.weapons[1]?.armed).toBeNull();
    // The VIEW composite carries the same keys.
    const view = decodeCockpitFrame(json("view_frame.json"));
    expect(view?.odds_floor_permille).toBe(100);
    expect(view?.weapons[0]?.armed?.slot).toBe(1);
  });

  it("decodes an older frame, without the keys, as it always did", () => {
    const old = decodeCockpitFrame({
      match: 3,
      callsign: "BB",
      weapons: [{ id: "gun", ready: true, rounds: 4, rounds_max: 10 }],
    });
    expect(old?.odds_floor_permille).toBeNull();
    expect(old?.weapons[0]?.armed).toBeNull();
    expect(old ? weaponLines(old) : []).toEqual(["gun  ready  4/10"]);
  });

  it("bounds and cleans an armed object as it does an odds entry", () => {
    const raw = json("cockpit_frame.json") as Record<string, unknown>;
    const weapons = raw.weapons as Record<string, unknown>[];
    const hostile = decodeCockpitFrame({
      ...raw,
      weapons: [
        {
          ...weapons[0],
          armed: {
            slot: 2,
            target: "be\u001bst_and_far_too_long",
            min_permille: 450,
            aspects: ["rear", "top", 7, "front"],
            keeps: -3,
            min_range_m: -1,
            max_range_m: "far",
            designator: "BA",
            contact: "00000000000000ab",
          },
        },
        { ...weapons[1], armed: "S1" },
        { ...weapons[1], armed: { target: "lock" } },
      ],
    });
    const armed = hostile?.weapons[0]?.armed;
    expect(armed).toEqual({
      slot: 2,
      target: "best_and",
      min_permille: 450,
      percent: 45,
      aspects: ["rear", "front"],
      keeps: 0,
      min_range_m: null,
      max_range_m: null,
    });
    // Nothing the decoder did not keep can reach a line.
    expect(Object.keys(armed ?? {})).not.toContain("designator");
    expect(Object.keys(armed ?? {})).not.toContain("contact");
    expect(JSON.stringify(hostile?.weapons)).not.toMatch(/[0-9a-f]{16}/);
    // Not an object, and an object with no slot: no arming.
    expect(hostile?.weapons[1]?.armed).toBeNull();
    expect(hostile?.weapons[2]?.armed).toBeNull();
    // An unknown target word prints nothing.
    expect(armed ? armingWords(armed) : "").toBe("S2: >=45% front/rear");
  });

  it("prints every term's words", () => {
    // The scene's: the lock, a flank or the rear, eight rounds kept.
    expect(armingWords(arming({ aspects: ["left", "right", "rear"], keeps: 8 }))).toBe(
      "S1: >=60% side/rear keeps 8",
    );
    expect(
      armingWords(
        arming({ aspects: ["left", "right", "rear"], keeps: 8, min_range_m: 200, max_range_m: 600 }),
      ),
    ).toBe("S1: >=60% side/rear 200-600m keeps 8");
    // T1's: the best hostile at Line's floor, under its admitting ceiling.
    expect(
      armingWords(arming({ target: "best", min_permille: 100, percent: 10, max_range_m: 420 })),
    ).toBe("S1: best >=10% under 420m");
    expect(armingWords(arming({ target: "contact", slot: 2 }))).toBe("S2: contact >=60%");
    expect(armingWords(arming({ min_range_m: 300 }))).toBe("S1: >=60% past 300m");
    // A zero lower bound is no bound.
    expect(armingWords(arming({ min_range_m: 0, max_range_m: 500 }))).toBe(
      "S1: >=60% under 500m",
    );
    expect(armingWords(arming({ aspects: ["front"] }))).toBe("S1: >=60% front");
    expect(armingWords(arming({ aspects: ["left"] }))).toBe("S1: >=60% side");
    expect(armingWords(arming({ aspects: ["front", "right", "rear"] }))).toBe(
      "S1: >=60% front/side/rear",
    );
    expect(armingWords(arming({ aspects: [] }))).toBe("S1: >=60%");
    expect(armingWords(arming({ keeps: 0 }))).toBe("S1: >=60%");
  });

  it("says the three new refusal words", () => {
    expect(notice({ kind: "refused", order: "fire", weapon: "cannon", refusal: "unwarranted" })).toBe(
      "FIRE cannon refused: no warrant",
    );
    expect(notice({ kind: "refused", order: "fire", weapon: "cannon", refusal: "no_slot" })).toBe(
      "FIRE cannon refused: every rule slot occupied",
    );
    expect(notice({ kind: "refused", order: "fire", weapon: "cannon", refusal: "unlawful" })).toBe(
      "FIRE cannon refused: that rule is not lawful for this unit",
    );
  });

  it("says the profile refusal word (17b cut M5.2)", () => {
    // The arena's word for a `MIND PROFILE` it cannot take; the order word is
    // the intent's own (`mind`), as every staged refusal's is.
    expect(notice({ kind: "refused", order: "mind", refusal: "profile" })).toBe(
      "MIND refused: that profile cannot be switched to now",
    );
  });
});

// **17b cut M5.5** (doc 17 §12, OQ-8; the M5 design note's M5.5 item 2): the
// keyframe's `mind` object (17b cut M5.4) decoded, and drawn as the weapons
// section's tail — `mindLines`, the TUI's `render_cockpit_mind_tail` line for
// line — in `(no weapon)`'s place on an unarmed hull.
describe("17b cut M5.5", () => {
  /** The canonical scene's tail (`cockpit_frame.json`'s `mind`). */
  const SCENE_TAIL = [
    "mind Standard Doctrine · Class I",
    "> Sniper   Line",
    "dormant while you fly",
    "you: helm sensors fire",
  ];

  /** The committed frame's raw object, a fresh copy each call. */
  function raw(): Record<string, unknown> {
    return json("cockpit_frame.json") as Record<string, unknown>;
  }

  function withMind(patch: Partial<CockpitMind>): CockpitFrame {
    const frame = theFrame();
    return { ...frame, mind: { ...frame.mind!, ...patch } };
  }

  it("decodes the committed mind object, and reads an absent or malformed one as null", () => {
    const frame = theFrame();
    expect(frame.mind).toEqual({
      name: "Standard Doctrine",
      card: {
        sections: expect.any(Array),
        values: {
          aspect: "any",
          close_m: 200,
          engage_band_permille: 1000,
          hunt_after_ms: 90_000,
          min_odds_permille: 650,
          opening: "wait",
          orbit_sense: "auto",
          prefer: "balanced",
          rally_ms: 20_000,
          reserve_odds_pct: 75,
          reserve_pct: 30,
          screen_arc: 120,
          screen_bearing: 0,
          search_ms: 60_000,
          stance: "hold",
          stand_min_permille: 175,
          stop_unseen: true,
          switch_m: 100,
          turn_into_hits: true,
          turn_s: 4,
          weight_facet: 40,
          weight_health: 30,
          weight_range: 100,
          withdraw_permille: 350,
        },
      },
      sealedRules: [],
      profiles: ["Sniper", "Line"],
      active: 0,
      class: "I",
      core: "II",
      channels: ["helm", "sensors", "fire"],
      next_switch_ms: null,
      off: false,
    });
    // The captured schema exposes every stored Sniper value at its station;
    // this is the real fixture's complete card, not a reconstructed preset.
    const sections = frame.mind!.card!.sections;
    expect(sections.map(({ id, label, station, fields }) => ({
      id, label, station, fields: fields.map((field) => field.id),
    }))).toEqual([
      {
        id: "posture", label: "Posture", station: "helm",
        fields: ["close_m", "engage_band_permille", "opening", "orbit_sense", "screen_arc",
          "screen_bearing", "stance", "turn_into_hits", "turn_s"],
      },
      {
        id: "fire", label: "Fire discipline", station: "fire",
        fields: ["aspect", "min_odds_permille", "reserve_odds_pct", "reserve_pct"],
      },
      {
        id: "targeting", label: "Targeting", station: "sensors",
        fields: ["prefer", "switch_m", "weight_facet", "weight_health", "weight_range"],
      },
      {
        id: "survival", label: "Survival and search", station: "helm",
        fields: ["hunt_after_ms", "rally_ms", "search_ms", "stand_min_permille", "stop_unseen",
          "withdraw_permille"],
      },
    ]);
    expect(sections.flatMap((section) => section.fields)
      .find((field) => field.id === "min_odds_permille")?.widget).toMatchObject({
        kind: "slider", min: 0, max: 1000, step: 1, scale: 10, unit: "%",
      });
    // An older server (no key), a hull no seal reached (`null`), and a value
    // that is not an object: no mind, and no tail.
    const old = raw();
    delete old.mind;
    for (const mind of [undefined, null, "Sniper", ["Sniper"]]) {
      const decoded = decodeCockpitFrame(mind === undefined ? old : { ...raw(), mind });
      expect(decoded?.mind).toBeNull();
      expect(decoded ? mindLines(decoded) : ["?"]).toEqual([]);
      expect(decoded ? mindProfiles(decoded) : ["?"]).toEqual([]);
    }
    // A profile that is not a string is dropped and the flown one is still the
    // one the wire's index named; the set is cut to four, the words cleaned.
    const odd = decodeMind({
      name: null,
      profiles: [7, "Line\u0007", "Sniper", "Sentry", "Hunter", "Rearguard"],
      active: 2,
      class: "II",
      core: "III",
      channels: ["helm", 3, "sensors"],
      next_switch_ms: 41_000,
      off: true,
    });
    expect(odd).toEqual({
      name: null,
      card: null,
      sealedRules: [],
      profiles: ["Line", "Sniper", "Sentry", "Hunter"],
      active: 1,
      class: "II",
      core: "III",
      channels: ["helm", "sensors"],
      next_switch_ms: 41_000,
      off: true,
    });
    expect(odd?.profiles).toHaveLength(MAX_MIND_PROFILES);
    expect(decodeMind({ profiles: [7, "Line"], active: 0 })?.active).toBe(-1);
  });

  it("draws the four lines under the weapons, in their order and words", () => {
    const frame = theFrame();
    expect(mindLines(frame)).toEqual(SCENE_TAIL);
    const view = cockpitView(frame, null, []);
    expect(view.weapons).toEqual(weaponLines(frame));
    expect(view.mind).toEqual(SCENE_TAIL);
    // The tail follows the weapon rows and comes before the contacts.
    const lines = cockpitViewLines(view);
    const gun = lines.indexOf("machine_gun  1 s  380/500  21%");
    expect(lines.slice(gun + 1, gun + 5)).toEqual(SCENE_TAIL);
    expect(lines[gun + 5]?.startsWith("BA  ")).toBe(true);
    // The profile row in its parts: the flown one marked, joined to the line.
    expect(mindProfiles(frame)).toEqual([
      { lead: "> ", name: "Sniper", active: true },
      { lead: "   ", name: "Line", active: false },
    ]);
    expect(view.profiles.map((p) => `${p.lead}${p.name}`).join("")).toBe(view.mind[1]);
  });

  it("says every reading, and draws no countdown", () => {
    // The other profile flown inside its switch window, a seal that is not
    // Standard Doctrine, a class read off the frame, a hull dropped to the ROM.
    const other = withMind({ active: 1, next_switch_ms: 40_000, name: null, class: "0", off: true });
    expect(mindLines(other)).toEqual([
      "mind: your own · Class 0",
      "  Sniper > Line",
      "off for the match: on the ROM",
      "you: helm sensors fire",
    ]);
    expect(mindLines({ ...other, mind: { ...other.mind!, next_switch_ms: null } })).toEqual(
      mindLines(other),
    );
    // A live class of 0 that is not `off` — a coreless hull such as
    // `target_mk1`, or a seal capped at Class 0 on a real core — is the ROM's
    // as well (line mode's `MIND` head names the ROM for both).
    for (const core of ["0", "II"]) {
      expect(mindLines(withMind({ class: "0", core, off: false }))).toEqual([
        "mind Standard Doctrine · Class 0",
        "> Sniper   Line",
        "off for the match: on the ROM",
        "you: helm sensors fire",
      ]);
    }
    expect(mindLines(withMind({ profiles: ["Line"], active: 0 }))[1]).toBe("> Line");
    expect(
      mindLines(withMind({ profiles: ["Sentry", "Sniper", "Skirmisher", "Rearguard"], active: 2 }))[1],
    ).toBe("  Sentry   Sniper > Skirmisher   Rearguard");
  });

  it("m5_the_mind_tail_takes_the_weapons_place_on_an_unarmed_hull", () => {
    // The drone: no weapons station, so no `weapons` key and no contact odds; a
    // Class I core; the unchosen slot's seal, Standard Doctrine on Line alone;
    // its channels `helm` and `sensors`.
    const drone = raw();
    delete drone.weapons;
    drone.rules = [];
    drone.contacts = (drone.contacts as Record<string, unknown>[]).map((c) => ({ ...c, odds: [] }));
    drone.mind = {
      name: "Standard Doctrine",
      profiles: ["Line"],
      active: 0,
      class: "I",
      core: "I",
      channels: ["helm", "sensors"],
      next_switch_ms: null,
      off: false,
    };
    const frame = decodeCockpitFrame(drone)!;
    expect(frame.armed).toBe(false);
    const tail = ["mind Standard Doctrine · Class I", "> Line", "dormant while you fly", "you: helm sensors"];
    const view = cockpitView(frame, null, []);
    expect(view.weapons).toEqual([]);
    expect(view.mind).toEqual(tail);
    const lines = cockpitViewLines(view);
    expect(lines).not.toContain("(no weapon)");
    const head = lines.indexOf(tail[0]!);
    expect(lines.slice(head, head + 4)).toEqual(tail);
    expect(lines[head - 1]).toBe(damageText(view.damage[view.damage.length - 1]!));
    // `weaponLines` itself is unchanged: the place is the view's to give.
    expect(weaponLines(frame)).toEqual(["(no weapon)"]);
    // With no mind on the frame, the unarmed hull reads as it did.
    const bare = cockpitView({ ...frame, mind: null }, null, []);
    expect(bare.weapons).toEqual(["(no weapon)"]);
    expect(bare.mind).toEqual([]);
    expect(cockpitViewLines(bare)).toContain("(no weapon)");
  });

  it("keeps every line within the TUI pane's 44 columns", () => {
    // Every ordered set of one to four of the six presets, each flown, either
    // name, every class word, either state, either channel list — the TUI's
    // `m5_every_mind_tail_row_is_at_most_44_columns`, over the same words.
    // A COPY of `tacsim::arena::card::PRESETS`: nothing ties it to the source,
    // so a renamed or longer preset would pass here unseen — tying both walks
    // to one committed fixture is carried to the cut that next touches the
    // presets (M6).
    const presets = ["Line", "Sentry", "Sniper", "Skirmisher", "Hunter", "Rearguard"];
    let sets: string[][] = [[]];
    const ordered: string[][] = [];
    for (let size = 0; size < 4; size += 1) {
      sets = sets.flatMap((set) => presets.filter((p) => !set.includes(p)).map((p) => [...set, p]));
      ordered.push(...sets);
    }
    expect(ordered).toHaveLength(6 + 30 + 120 + 360);
    const frame = theFrame();
    const base = frame.mind!;
    let widest = 0;
    let checked = 0;
    const over: string[] = [];
    for (const profiles of ordered) {
      for (let active = 0; active < profiles.length; active += 1) {
        for (const name of ["Standard Doctrine", null]) {
          for (const cls of ["0", "I", "II", "III"]) {
            for (const off of [false, true]) {
              for (const channels of [["helm", "sensors", "fire"], ["helm", "sensors"]]) {
                const mind = { ...base, profiles, active, name, class: cls, off, channels };
                for (const line of mindLines({ ...frame, mind })) {
                  widest = Math.max(widest, line.length);
                  if (line.length > 44) {
                    over.push(line);
                  }
                }
                checked += 1;
              }
            }
          }
        }
      }
    }
    expect(over).toEqual([]);
    expect(checked).toBe((6 + 2 * 30 + 3 * 120 + 4 * 360) * 2 * 4 * 2 * 2);
    expect(widest).toBe(42);
  });
});
