// The replay's pure half (16b cut I.2; 16c N9, Q37): the decoders, the frame
// between keyframes, the one composition and the view model — over the
// committed fixture set `fixtures/replay/`, whose `replay_grid.txt` is the byte
// target this file's `composeReplayWindow` and the TUI's replay map both equal.
//
// `replay_view_model.txt` is this file's own golden; re-bless it with
// `REPLAY_BLESS=1 npx vitest run src/lib/replay.test.ts`. The JSON files are
// blessed by `crates/tacsim/tests/replay.rs` (`TACSIM_BLESS=1`), the one
// blesser, and `replay_grid.txt` by `mush-ansi`'s golden.

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { decodeArenaMap, markerLegend, type ArenaMap } from "./cockpit";
import {
  composeReplayWindow,
  decodeObserver,
  decodeReplay,
  decodeReplayGrid,
  frameAt,
  isObserverCleared,
  LIVE_LABEL,
  liveAsReplay,
  liveView,
  modeTag,
  outcomeWords,
  replayView,
  replayViewLines,
  type Replay,
} from "./replay";

const DIR = "src/lib/fixtures/replay";

function text(dir: string, name: string): string {
  return readFileSync(resolve(process.cwd(), dir, name), "utf8");
}

/** The document as the query answers it: under its own `replay` shape. */
function answer(): Record<string, unknown> {
  const doc = JSON.parse(text(DIR, "replay.json")) as Record<string, unknown>;
  return { schema: { id: "arena.replay", version: 1, shape: "replay" }, ...doc };
}

function theReplay(): Replay {
  const replay = decodeReplay(answer());
  if (replay === null) {
    throw new Error("the committed document decodes");
  }
  return replay;
}

function theMap(): ArenaMap {
  const map = decodeArenaMap(JSON.parse(text("src/lib/fixtures/cockpit", "arena_map.json")));
  if (map === null) {
    throw new Error("the committed map decodes");
  }
  return map;
}

describe("decodeReplay", () => {
  it("reads the committed document whole", () => {
    const r = theReplay();
    expect(r.match).toBe(7);
    expect(r.roster.map((u) => `${u.callsign} ${u.team}`)).toEqual(["RA red", "RB red", "BA blue", "BB blue"]);
    expect(r.keyframes.map((k) => k.t_ms)).toEqual([0, 5000, 10000]);
    expect(r.keyframes[1]?.units[3]).toEqual({ x: 14, y: 7, heading_deg: 225 });
    expect(r.shots).toEqual([
      { t_ms: 6000, shooter: 0, target: 2, outcome: "hit" },
      { t_ms: 8000, shooter: 2, target: 0, outcome: "miss" },
    ]);
    expect(r.destroyed).toEqual([[7000, 3]]);
    expect(r.end_ms).toBe(10000);
    expect(r.truncated).toBe(false);
  });

  it("is tolerant of missing, extra and malformed keys", () => {
    // Extra keys are ignored; missing ones read empty.
    const sparse = decodeReplay({ schema: { shape: "replay" }, match: 3, surprise: { a: 1 } });
    expect(sparse).toMatchObject({ match: 3, roster: [], keyframes: [], shots: [], destroyed: [], end_ms: 0 });
    // A slot that is not three numbers reads `null`; a short keyframe is padded
    // to the roster; a shot or wreck past the roster is skipped; an unreadable
    // side reads red; an unknown cut word is dropped.
    const odd = decodeReplay({
      schema: { shape: "replay" },
      match: 4,
      roster: [{ callsign: "RA\u0007", team: "blue" }, { callsign: "BB", team: 7 }],
      keyframes: [[0, [[1, 2, 370], "x"]], [5000], "junk", [-1, []]],
      shots: [[1, 0, 1, "hit"], [2, 0, 5, "hit"], [3, 0, 1, "maybe"]],
      destroyed: [[4, 1], [5, 9]],
      cut: ["keyframes", "gremlins"],
      truncated: true,
    });
    expect(odd?.roster).toEqual([
      { callsign: "RA", team: "blue" },
      { callsign: "BB", team: "red" },
    ]);
    expect(odd?.keyframes).toEqual([
      { t_ms: 0, units: [{ x: 1, y: 2, heading_deg: 10 }, null] },
      { t_ms: 5000, units: [null, null] },
    ]);
    expect(odd?.shots).toEqual([{ t_ms: 1, shooter: 0, target: 1, outcome: "hit" }]);
    expect(odd?.destroyed).toEqual([[4, 1]]);
    expect(odd?.cut).toEqual(["keyframes"]);
    // The stored form, with no schema, reads by its `record`.
    expect(decodeReplay(JSON.parse(text(DIR, "replay.json")))?.match).toBe(7);
    // `available: false`, the pushed grid and junk are not documents.
    expect(
      decodeReplay({ schema: { shape: "replay" }, available: false, reason: "You have no replay open; type SIM REPLAY <match>." }),
    ).toBeNull();
    expect(decodeReplay(JSON.parse(text(DIR, "replay_window.json")))).toBeNull();
    for (const junk of [null, 7, "x", [], { schema: { shape: "replay" } }]) {
      expect(decodeReplay(junk)).toBeNull();
    }
  });

  it("tells the pushed grid by its shape, and reads its match and keyframe", () => {
    expect(decodeReplayGrid(JSON.parse(text(DIR, "replay_window.json")))).toEqual({ match: 7, keyframe: 1 });
    expect(decodeReplayGrid(answer())).toBeNull();
    expect(decodeReplayGrid({ schema: { shape: "grid" } })).toBeNull();
  });
});

// **16b cut O.1 (16c N11; plan item 39)**: a hold's words, and the mode's tag
// on the replay's header — nothing for a skirmish or a document with no `mode`
// (every replay written before the cut).
describe("the mode", () => {
  it("words a hold and tags the header", () => {
    expect(outcomeWords("red", "hold")).toBe("red won (red held the key spot)");
    expect(outcomeWords("blue", "hold")).toBe("blue won (blue held the key spot)");
    expect(modeTag("hold")).toBe(" [hold]");
    expect(modeTag("skirmish")).toBe("");
    expect(modeTag("")).toBe("");
    // The committed document is a skirmish; a hold of the same tape heads
    // itself with the tag after the outcome words.
    const skirmish = theReplay();
    expect(skirmish.mode).toBe("skirmish");
    const held = decodeReplay({ ...answer(), outcome: "red", reason: "hold", mode: "hold" });
    if (held === null) {
      throw new Error("the hold decodes");
    }
    expect(held.mode).toBe("hold");
    const frame = frameAt(held, 5000);
    expect(replayView(held, frame, null, 5000).header).toBe(
      "Replay of match 7 — red won (red held the key spot) [hold] — both sides are shown: this match is over.",
    );
    expect(replayView(skirmish, frameAt(skirmish, 5000), null, 5000).header).not.toContain("[");
    // A document with no `mode` reads as none, and tags nothing.
    const old = answer();
    delete old.mode;
    expect(decodeReplay(old)?.mode).toBe("");
  });
});

describe("frameAt", () => {
  it("is the keyframe itself on a keyframe's own time", () => {
    const f = frameAt(theReplay(), 5000);
    expect(f).toMatchObject({ t_ms: 5000, between: false, held: false, before: 1, after: 1 });
    // The first and the last on their own times are keyframes too, not held.
    expect(frameAt(theReplay(), 0)).toMatchObject({ between: false, held: false, before: 0, after: 0 });
    expect(frameAt(theReplay(), 10_000)).toMatchObject({ between: false, held: false, before: 2, after: 2 });
    expect(f.units[0]).toEqual({ x: 6, y: 4, heading_deg: 90 });
  });

  it("moves each unit in a line between two, rounded, and turns the shorter way", () => {
    const f = frameAt(theReplay(), 2500);
    expect(f).toMatchObject({ between: true, held: false, before: 0, after: 1 });
    // RA (4,4) → (6,4): the midpoint.
    expect(f.units[0]).toEqual({ x: 5, y: 4, heading_deg: 90 });
    // RB (2,7) heading 90 → (2,9) heading 0: y 8, heading 45.
    expect(f.units[1]).toEqual({ x: 2, y: 8, heading_deg: 45 });
    // BA (40,14) → (36,14): x 38.
    expect(f.units[2]).toEqual({ x: 38, y: 14, heading_deg: 270 });
    // BB (16,7) 270 → (14,7) 225: x 15, heading 248 (247.5 rounded).
    expect(f.units[3]).toEqual({ x: 15, y: 7, heading_deg: 248 });
    // A quarter of the way, the cells round to the nearest.
    expect(frameAt(theReplay(), 1250).units[2]).toEqual({ x: 39, y: 14, heading_deg: 270 });
  });

  it("crosses north along the shorter arc, never through 180°", () => {
    const r: Replay = {
      ...theReplay(),
      roster: [{ callsign: "RA", team: "red" }],
      keyframes: [
        { t_ms: 0, units: [{ x: 0, y: 0, heading_deg: 350 }] },
        { t_ms: 1000, units: [{ x: 0, y: 0, heading_deg: 10 }] },
      ],
    };
    expect(frameAt(r, 500).units[0]?.heading_deg).toBe(0);
    expect(frameAt(r, 250).units[0]?.heading_deg).toBe(355);
    expect(frameAt(r, 750).units[0]?.heading_deg).toBe(5);
    // And the other way round.
    const back: Replay = {
      ...r,
      keyframes: [
        { t_ms: 0, units: [{ x: 0, y: 0, heading_deg: 10 }] },
        { t_ms: 1000, units: [{ x: 0, y: 0, heading_deg: 350 }] },
      ],
    };
    expect(frameAt(back, 500).units[0]?.heading_deg).toBe(0);
  });

  it("clamps before the first keyframe and after the last, HELD, and draws a missing unit where it was", () => {
    const r = theReplay();
    // Off every keyframe's own time: not the arena's positions at `t_ms`, so
    // `between` — and `held`, the one keyframe on both sides.
    expect(frameAt(r, -100)).toMatchObject({ between: true, held: true, before: 0, after: 0 });
    expect(frameAt(r, -100).units[0]).toEqual({ x: 4, y: 4, heading_deg: 90 });
    const late = frameAt(r, 99_000);
    expect(late).toMatchObject({ between: true, held: true, before: 2, after: 2 });
    expect(late.units[0]).toEqual({ x: 9, y: 4, heading_deg: 90 });
    const gap: Replay = {
      ...r,
      keyframes: [
        { t_ms: 0, units: [{ x: 1, y: 1, heading_deg: 0 }, null, null, null] },
        { t_ms: 1000, units: [null, { x: 5, y: 5, heading_deg: 0 }, null, null] },
      ],
    };
    const mid = frameAt(gap, 500);
    expect(mid.units).toEqual([{ x: 1, y: 1, heading_deg: 0 }, { x: 5, y: 5, heading_deg: 0 }, null, null]);
  });
});

describe("composeReplayWindow", () => {
  // **The parity** (doc 16 §5): if this fails, the first differing cell is a
  // compositor defect, never a golden to re-bless.
  it("composes the committed grid at exactly 39 by 20 cells", () => {
    const r = theReplay();
    const win = composeReplayWindow(theMap(), r, frameAt(r, 5000), 0, 39, 20);
    expect(win).toMatchObject({ x0: 0, y0: 0, w: 39, h: 20 });
    const expected = text(DIR, "replay_grid.txt").replace(/\r\n/g, "\n").replace(/\n$/, "");
    const got = win.grid.join("\n");
    if (got !== expected) {
      const want = expected.split("\n");
      const have = got.split("\n");
      for (let i = 0; i < Math.max(want.length, have.length); i += 1) {
        expect(have[i], `row ${i} (north-up) differs; that is a compositor defect`).toBe(want[i]);
      }
    }
    expect(got).toBe(expected);
    // Spots first, then the roster in order; BB covers The Roof.
    expect(win.markers.map((m) => `${m.glyph} ${m.designator}`)).toEqual([
      "A The Ford",
      "B The Roof",
      "o RA",
      "o RB",
      "* BA",
      "* BB",
    ]);
    expect(markerLegend(win)).toBe("o RA  o RB  * BA  * BB  A The Ford");
  });

  it("draws a wreck from the moment it was recorded, and centres on the focus", () => {
    const r = theReplay();
    const before = composeReplayWindow(theMap(), r, frameAt(r, 6999), 0, 39, 20);
    expect(before.markers.find((m) => m.designator === "BB")?.kind).toBe("blue");
    const after = composeReplayWindow(theMap(), r, frameAt(r, 7000), 0, 39, 20);
    const bb = after.markers.find((m) => m.designator === "BB");
    expect(bb).toMatchObject({ glyph: "%", kind: "wreck" });
    expect(markerLegend(after)).toContain("% BB");
    // Focused on BA at (36, 14), a 20-cell window slides east.
    const onBa = composeReplayWindow(theMap(), r, frameAt(r, 5000), 2, 20, 10);
    expect(onBa).toMatchObject({ x0: 26, y0: 9, w: 20, h: 10 });
  });
});

describe("the view model", () => {
  it("says both sides, and says so when it is drawing between keyframes", () => {
    const r = theReplay();
    const on = replayView(r, frameAt(r, 5000), null, 5000);
    expect(on.header).toBe("Replay of match 7 — aborted — both sides are shown: this match is over.");
    expect(on.label).toBe("Keyframe 2 of 3 (0:05): the arena's own positions.");
    const off = replayView(r, frameAt(r, 6500), null, 6500);
    expect(off.label).toBe("Between keyframes 2 and 3: drawn, not recorded.");
    // Past the last keyframe — `end_ms` usually is, the arena writing no
    // summary at the end — the last keyframe is HELD, and says so: never the
    // keyframe's own sentence at a time that is not its own.
    const long: Replay = { ...r, end_ms: 12_000 };
    const held = replayView(long, frameAt(long, 12_000), null, 12_000);
    expect(held.label).toBe("After keyframe 3 (0:10): the last recorded positions, held.");
    expect(held.clock).toBe("0:12 of 0:12");
    const early: Replay = { ...r, keyframes: r.keyframes.map((kf) => ({ ...kf, t_ms: kf.t_ms + 1000 })) };
    expect(replayView(early, frameAt(early, 0), null, 0).label).toBe(
      "Before keyframe 1 (0:01): the first recorded positions, held.",
    );
    expect(off.shots).toEqual(["0:06  RA at BA: hit"]);
    expect(replayView(r, frameAt(r, 9000), null, 9000).wrecks).toEqual(["0:07  BB wrecked"]);
    expect(replayView(r, frameAt(r, 9000), null, 9000).shots).toEqual([
      "0:08  BA at RA: miss",
      "0:06  RA at BA: hit",
    ]);
    // A cut replay says what it holds, one sentence a cause.
    const cut = replayView({ ...r, truncated: true, cut: ["keyframes", "shots"] }, frameAt(r, 0), null, 0);
    expect(cut.truncation).toEqual([
      "The replay holds the first 10 s of match 7; the tape has the rest.",
      "The replay keeps the shots of match 7 to 0:08; the tape has the rest.",
    ]);
  });

  // The golden: every line the screen prints at keyframe 1, composed at the
  // parity window. The component renders these strings and decides none.
  it("matches the committed view-model golden", () => {
    const r = theReplay();
    const frame = frameAt(r, 5000);
    const win = composeReplayWindow(theMap(), r, frame, 0, 39, 20);
    const lines = replayViewLines(replayView(r, frame, win, 5000));
    const path = resolve(process.cwd(), DIR, "replay_view_model.txt");
    const body = `${lines.join("\n")}\n`;
    if (process.env.REPLAY_BLESS === "1") {
      writeFileSync(path, body, "utf8");
    }
    expect(body).toBe(readFileSync(path, "utf8").replace(/\r\n/g, "\n"));
    // Both sides, by callsign, and nothing a document does not carry.
    for (const callsign of ["RA", "RB", "BA", "BB"]) {
      expect(body).toContain(callsign);
    }
    expect(body).toContain("both sides are shown");
    expect(body).not.toMatch(/#\d/);
  });
});

// The live picture (16b cut I.4; 16c N10): one `arena.observer` keyframe, read,
// drawn as a one-keyframe replay and worded — a staff surface, so the object
// numbers are on it on purpose.
describe("the live picture", () => {
  const live = (): Record<string, unknown> => ({
  schema: { id: "arena.observer", version: 1, retain: "latest" },
  match: 7,
  now_ms: 90_000,
  clock: "1:30",
  tempo: "engaged",
  floor_ms: 5_000,
  cell_cm: 1_000,
  width: 96,
  height: 64,
  in_flight: 2,
  units: [
    ["RA", "R", 6, 4, 90, 301, "pm"],
    ["RB", "R", null, null, 0, 303, "s"],
    ["BA", "B", 20, 14, 270, 302, "ph"],
    ["BB", "B", 14, 7, 225, 304, "w"],
  ],
});

  it("reads a picture whole, and tells `watching: false` from one", () => {
    const pic = decodeObserver(live());
    expect(pic).not.toBeNull();
    expect(pic).toMatchObject({ match: 7, now_ms: 90_000, tempo: "engaged", floor_ms: 5_000, in_flight: 2 });
    expect(pic?.units.map((u) => [u.callsign, u.team, u.cell, u.id, u.flags])).toEqual([
      ["RA", "red", [6, 4], 301, "pm"],
      ["RB", "red", null, 303, "s"],
      ["BA", "blue", [20, 14], 302, "ph"],
      ["BB", "blue", [14, 7], 304, "w"],
    ]);
    const cleared = { schema: { id: "arena.observer", version: 1, retain: "latest" }, watching: false };
    expect(decodeObserver(cleared)).toBeNull();
    expect(isObserverCleared(cleared)).toBe(true);
    expect(isObserverCleared(live())).toBe(false);
    expect(decodeObserver({ units: [] })).toBeNull();
  });

  it("draws the units as a replay draws them: a wreck `%`, a unit with no cell nowhere", () => {
    const pic = decodeObserver(live())!;
    const doc = liveAsReplay(pic);
    const frame = frameAt(doc, pic.now_ms);
    expect(frame.between).toBe(false);
    const win = composeReplayWindow(theMap(), doc, frame, 0, 39, 20);
    const at = (x: number, y: number): string => win.grid[win.h - 1 - (y - win.y0)]?.charAt(x - win.x0) ?? "";
    expect(at(6, 4)).toBe("o");
    expect(at(20, 14)).toBe("*");
    expect(at(14, 7)).toBe("%");
    expect(win.markers.some((m) => m.designator === "RB")).toBe(false);
  });

  it("words the picture: live, at most once an arena second, every unit by its number", () => {
    const pic = decodeObserver(live())!;
    const doc = liveAsReplay(pic);
    const win = composeReplayWindow(theMap(), doc, frameAt(doc, pic.now_ms), 0, 39, 20);
    const view = liveView(pic, win);
    expect(view.header).toBe("Watching 7 — 1:30 (live)");
    expect(view.label).toBe(LIVE_LABEL);
    expect(view.label).toBe("Live: the arena's own positions, at most once an arena second.");
    expect(view.clock).toBe("1:30");
    expect(view.units.map((u) => `${u.callsign.padEnd(4)} ${u.side.padEnd(5)} ${u.where}`)).toEqual([
      "RA   red   (6, 4)  090°  #301  machine",
      "RB   red   off the window  #303  pilot  starting",
      "BA   blue  (20, 14)  270°  #302  pilot  held",
      "BB   blue  (14, 7)  225°  #304  pilot  wreck",
    ]);
    expect(view.status).toEqual([
      "Match 7  engaged  floor 5000",
      "Live, at most once a second.",
      "In flight: 2",
      "@arena/watch off stops it.",
    ]);
    for (const line of view.status) {
      expect([...line].length).toBeLessThanOrEqual(32);
    }
    expect(view.shots).toEqual([]);
    expect(liveView({ ...pic, floor_ms: null }, win).status[0]).toBe("Match 7  engaged");
    // A replay has no status lines, so its golden is unchanged.
    const r = theReplay();
    expect(replayView(r, frameAt(r, 0), null, 0).status).toEqual([]);
  });
});
