// The cockpit's pure half (16b cut Q.5; doc 16 §5): the two decoders, the ONE
// map composition, and the view model the screen prints.
//
// The load-bearing assertion here is `the parity golden`: this client's
// composition of `fixtures/cockpit/arena_map.json` + `cockpit_frame.json` at
// exactly 78 by 20 equals `view_grid.txt` byte for byte — the same file the
// TUI's `tui_golden.rs` and `mush-ansi`'s `golden.rs` are held to. Three
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
  cockpitView,
  cockpitViewLines,
  composeWindow,
  contactText,
  damageText,
  decodeArenaMap,
  decodeCockpitFrame,
  labelledAt,
  labelPlan,
  noticeLine,
  noticeLines,
  scanChips,
  scanText,
  statusLines,
  weaponLines,
  type ArenaMap,
  type CockpitFrame,
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
  it("composes the committed grid at exactly 78 by 20", () => {
    const win = composeWindow(theMap(), theFrame(), 78, 20);
    expect(win).toMatchObject({ x0: 0, y0: 0, w: 78, h: 20 });
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
    expect(at(14, 7)).toBe("?"); // beats the building it stands on
    expect(at(13, 7)).toBe("#");
    // The marker-free rows keep the building under the contact.
    expect(win.terrain[win.h - 1 - 7]?.charAt(14)).toBe("#");
    // X4 at (50, 33) is off a twenty-row window: not drawn, not in the legend,
    // still in the table.
    expect(win.markers.map((m) => m.designator)).toEqual(["RA", "BA", "RB", "X3"]);
    expect(cockpitView(theFrame(), win, []).legend).toBe("x BA  + RB  ? X3");
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
    expect(win.markers.map((m) => m.designator)).toEqual(["RA", "BA", "X3", "RB", "X5"]);
    expect(win.grid[win.h - 1 - 9]?.charAt(2)).toBe("+");
    expect(win.grid[win.h - 1 - 4]?.charAt(6)).toBe("?");
    expect(cockpitView(stacked, win, []).legend).toBe("? X3  + RB  ? X5");
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
    // 450 m lock reads `far`.
    expect(view.contacts.map(contactText)[0]).toBe("BA  blue  045°  far  450 m  1 s");
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
    expect(weaponLines({ ...frame, weapons: [] })).toEqual(["(no weapon)"]);
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
    // The committed frame prints neither.
    for (const line of statusLines(frame)) {
      expect(line).not.toMatch(/receiver|DESTROYED|IMMOBILE/);
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
  });
});
