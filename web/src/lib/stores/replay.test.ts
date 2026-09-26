// ReplayStore (16b cut I.2): the one-shot document ask, the map latch, the
// dismissal keyed on the envelope seq, `available: false` closing the picture,
// and the re-hydrate that keeps the picture open and asks again.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { MAP_ASK_MS } from "./cockpit.svelte";
import { LIVE_STALE_MS, REPLAY_ASK_MS, ReplayStore } from "./replay.svelte";

const read = (dir: string, name: string): Record<string, unknown> =>
  JSON.parse(readFileSync(resolve(process.cwd(), dir, name), "utf8")) as Record<string, unknown>;

const grid = (): Record<string, unknown> => read("src/lib/fixtures/replay", "replay_window.json");
const doc = (): Record<string, unknown> => ({
  schema: { id: "arena.replay", version: 1, shape: "replay" },
  ...read("src/lib/fixtures/replay", "replay.json"),
});
const ground = (): Record<string, unknown> => read("src/lib/fixtures/cockpit", "arena_map.json");
const unavailable = {
  schema: { id: "arena.replay", version: 1, shape: "replay" },
  available: false,
  reason: "You have no replay open; type SIM REPLAY <match>.",
};

describe("ReplayStore", () => {
  it("opens on a grid and asks the document ONCE until it is answered", () => {
    const s = new ReplayStore();
    expect(s.needsDocument(0)).toBe(false); // nothing open
    expect(s.applyGrid(grid(), 10)).toBe(true);
    expect(s).toMatchObject({ open: true, match: 7, doc: null, focus: 0, live: false });
    expect(s.needsDocument(0)).toBe(true);
    expect(s.docAsked).toBe(true);
    // Outstanding: another grid of the match, and every later check, asks
    // nothing — there is no timer on the document.
    expect(s.applyGrid(grid(), 11)).toBe(false);
    expect(s.needsDocument(1)).toBe(false);
    expect(s.needsDocument(99 * REPLAY_ASK_MS)).toBe(false);
    s.applyDocument(doc());
    expect(s.docAsked).toBe(false);
    expect(s.doc?.match).toBe(7);
    // The cursor opens on the keyframe the grid showed (index 1, 0:05).
    expect(s.cursorMs).toBe(5000);
    expect(s.needsDocument(2)).toBe(false);
    // `p` asks it again — and once.
    s.pull(3);
    expect(s.needsDocument(3)).toBe(true);
    expect(s.needsDocument(3)).toBe(false);
  });

  it("`p` never re-asks while an ask is outstanding, until the latch presumes it dropped", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    const at = 100_000;
    expect(s.needsDocument(at)).toBe(true);
    // Mashed on a slow link while the first answer is in flight: nothing more.
    for (const t of [at, at + 1, at + REPLAY_ASK_MS - 1]) {
      s.pull(t);
      expect(s.needsDocument(t)).toBe(false);
    }
    // The answer landing does not fire a pull made while it was in flight.
    s.applyDocument(doc());
    expect(s.needsDocument(at + 5)).toBe(false);
    // Asked again; no answer comes (a throttled ask gets none): past the latch
    // `p` asks once more — and once.
    s.pull(at + 10);
    expect(s.needsDocument(at + 10)).toBe(true);
    s.pull(at + 10 + REPLAY_ASK_MS);
    expect(s.needsDocument(at + 10 + REPLAY_ASK_MS)).toBe(true);
    expect(s.needsDocument(at + 10 + REPLAY_ASK_MS)).toBe(false);
  });

  it("asks the ground under the 2 s latch until it is held, and drops another match's", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    expect(s.needsMap(100_000)).toBe(true);
    expect(s.needsMap(100_000 + MAP_ASK_MS - 1)).toBe(false);
    expect(s.needsMap(100_000 + MAP_ASK_MS)).toBe(true);
    s.applyMap({ ...ground(), match: 9 });
    expect(s.map).toBeNull();
    s.applyMap({ schema: { id: "arena.map" }, seated: false });
    expect(s.map).toBeNull();
    s.applyMap(ground());
    expect(s.holdsTheGround).toBe(true);
    expect(s.needsMap(999_999)).toBe(false);
  });

  it("drops a document of another match, and one with nothing open", () => {
    const s = new ReplayStore();
    s.applyDocument(doc());
    expect(s.doc).toBeNull();
    s.applyGrid({ ...grid(), match: 8 }, 3);
    s.applyDocument(doc());
    expect(s.doc).toBeNull();
    expect(s.open).toBe(true);
  });

  it("closes and dismisses on `available: false`", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    s.needsDocument(0);
    s.applyDocument(unavailable);
    expect(s.open).toBe(false);
    expect(s.dismissed.has(7)).toBe(true);
    expect(s.docAsked).toBe(false);
    // With nothing open it dismisses nothing.
    const idle = new ReplayStore();
    idle.applyDocument(unavailable);
    expect(idle.dismissed.size).toBe(0);
  });

  it("stays closed against a replayed grid and reopens on a fresh one", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    s.applyDocument(doc());
    s.applyMap(ground());
    s.close();
    expect(s).toMatchObject({ open: false, match: null, doc: null, map: null });
    expect(s.dismissed.has(7)).toBe(true);
    // The resume ring replays the retained grid with the seq it was sent with.
    expect(s.applyGrid(grid(), 10)).toBe(false);
    expect(s.open).toBe(false);
    expect(s.applyGrid(grid(), 9)).toBe(false);
    // A fresh `SIM REPLAY 7`: a newer seq clears the mark.
    expect(s.applyGrid(grid(), 11)).toBe(true);
    expect(s.dismissed.has(7)).toBe(false);
    expect(s.needsDocument(0)).toBe(true);
  });

  it("forgets every seq and every mark on a fresh attach, whose seqs count from 1 again", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 5000);
    s.close();
    expect(s.dismissed.has(7)).toBe(true);
    // Before the seam a low seq is a replayed grid…
    expect(s.applyGrid(grid(), 40)).toBe(false);
    // …after it, the new attach's numbering: a fresh `SIM REPLAY 7` at seq 40
    // opens the closed match again.
    s.beginSegment();
    expect(s.dismissed.size).toBe(0);
    expect(s.applyGrid(grid(), 40)).toBe(true);
    // And an open picture's old seq is not what its next dismissal records.
    s.close();
    expect(s.applyGrid(grid(), 40)).toBe(false);
    expect(s.applyGrid(grid(), 41)).toBe(true);
  });

  it("keeps the picture across a re-hydrate and asks for both again", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    expect(s.needsDocument(0)).toBe(true);
    s.applyDocument(doc());
    s.applyMap(ground());
    expect(s.needsMap(100)).toBe(false);
    s.stepKeyframe(1);
    s.clearForRehydrate();
    expect(s).toMatchObject({ open: true, match: 7, doc: null, map: null, cursorMs: 10000 });
    expect(s.needsDocument(1)).toBe(true);
    expect(s.needsMap(101)).toBe(true);
    // The answer lands on the cursor the viewer left.
    s.applyDocument(doc());
    expect(s.cursorMs).toBe(10000);
  });

  it("steps keyframes from the cursor, seeks inside the replay and walks the focus", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    s.applyDocument(doc());
    s.stepKeyframe(1);
    expect(s.cursorMs).toBe(10000);
    s.stepKeyframe(1);
    expect(s.cursorMs).toBe(10000); // the last is the last
    s.seek(6500);
    s.stepKeyframe(-1);
    expect(s.cursorMs).toBe(5000); // from between two, to the earlier
    s.seek(6500);
    s.stepKeyframe(1);
    expect(s.cursorMs).toBe(10000);
    s.stepKeyframe(-1);
    s.stepKeyframe(-1);
    s.stepKeyframe(-1);
    expect(s.cursorMs).toBe(0);
    s.seek(-5);
    expect(s.cursorMs).toBe(0);
    s.seek(99_999);
    expect(s.cursorMs).toBe(10000);
    s.playing = true;
    s.stepKeyframe(-1);
    expect(s.playing).toBe(false);
    for (const want of [1, 2, 3, 0]) {
      s.nextFocus();
      expect(s.focus).toBe(want);
    }
  });

  it("`.` past the last keyframe stays put, and `,` there steps back to it", () => {
    // The scrubber runs to `end_ms`, usually after the last keyframe (the arena
    // writes no summary at the end): `next` from there must not step BACK.
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    s.applyDocument({ ...doc(), end_ms: 12_000 });
    s.seek(12_000);
    s.stepKeyframe(1);
    expect(s.cursorMs).toBe(12_000);
    s.stepKeyframe(-1);
    expect(s.cursorMs).toBe(10_000);
    s.stepKeyframe(1);
    expect(s.cursorMs).toBe(10_000); // on the last, `next` has nowhere to go
  });
});

// The live picture (16b cut I.4; 16c N10): the same store in LIVE mode.
describe("ReplayStore, live", () => {
  const live = (match = 7, now_ms = 90_000): Record<string, unknown> => ({
    ...({
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
}),
    match,
    now_ms,
  });
  const cleared = { schema: { id: "arena.observer", version: 1, retain: "latest" }, watching: false };

  it("opens live on a picture, asks no document, and a later picture replaces it", () => {
    const s = new ReplayStore();
    expect(s.applyObserver(live(), 5, 0)).toBe(true);
    expect(s).toMatchObject({ open: true, live: true, match: 7, focus: 0, cursorMs: 90_000 });
    expect(s.doc?.roster.map((u) => u.callsign)).toEqual(["RA", "RB", "BA", "BB"]);
    expect(s.needsDocument(0)).toBe(false);
    expect(s.needsMap(0)).toBe(true);
    expect(s.applyObserver(live(7, 91_000), 6, 0)).toBe(false);
    expect(s.cursorMs).toBe(91_000);
    expect(s.observer?.now_ms).toBe(91_000);
    // An answer on the replay feed is not a live picture's to take.
    s.applyDocument(doc());
    expect(s.doc?.keyframes).toHaveLength(1);
  });

  it("nothing steps or scrubs it; `u` walks its units; `p` owes one `arena.observer` ask", () => {
    const s = new ReplayStore();
    s.applyObserver(live(), 5, 0);
    s.stepKeyframe(1);
    s.stepKeyframe(-1);
    s.seek(0);
    expect(s.cursorMs).toBe(90_000);
    s.nextFocus();
    expect(s.focus).toBe(1);
    expect(s.needsObserver(0)).toBe(false);
    s.pull(0);
    expect(s.needsObserver(0)).toBe(true);
    expect(s.needsObserver(0)).toBe(false);
    expect(s.needsDocument(0)).toBe(false);
  });

  it("closes an open replay: the picture replaced, the replay's match dismissed", () => {
    const s = new ReplayStore();
    s.applyGrid(grid(), 10);
    s.applyDocument(doc());
    s.applyMap(ground());
    expect(s.applyObserver(live(12), 11, 0)).toBe(true);
    expect(s).toMatchObject({ open: true, live: true, match: 12, map: null });
    expect(s.dismissed.has(7)).toBe(true);
    // The same match's ground is kept.
    const t = new ReplayStore();
    t.applyObserver(live(7), 1, 0);
    t.applyMap(ground());
    expect(t.holdsTheGround).toBe(true);
    t.applyObserver(live(7, 91_000), 2, 0);
    expect(t.holdsTheGround).toBe(true);
  });

  it("`watching: false` closes it and dismisses the match; a replay is left alone", () => {
    const s = new ReplayStore();
    s.applyObserver(live(), 5, 0);
    s.applyObserver(cleared, 6, 0);
    expect(s).toMatchObject({ open: false, live: false, match: null, observer: null });
    expect(s.dismissed.has(7)).toBe(true);
    // A picture replayed out of the resume ring does not re-open it; a new
    // watch — a newer seq — does.
    expect(s.applyObserver(live(), 5, 0)).toBe(false);
    expect(s.open).toBe(false);
    expect(s.applyObserver(live(), 9, 0)).toBe(true);
    expect(s.dismissed.has(7)).toBe(false);

    const r = new ReplayStore();
    r.applyGrid(grid(), 10);
    r.applyObserver(cleared, 11, 0);
    expect(r).toMatchObject({ open: true, live: false, match: 7 });
  });

  it("a picture gone `LIVE_STALE_MS` quiet is owed one ask, latched; a healthy `Quiet` watch never is", () => {
    const s = new ReplayStore();
    s.applyObserver(live(), 5, 100_000);
    expect(s.needsObserver(100_000 + LIVE_STALE_MS - 1)).toBe(false);
    // A picture every five seconds (`Quiet`) or ten (the longest floor).
    s.applyObserver(live(7, 95_000), 6, 105_000);
    expect(s.needsObserver(115_000)).toBe(false);
    s.applyObserver(live(7, 105_000), 7, 115_000);
    expect(s.needsObserver(115_000 + LIVE_STALE_MS - 1)).toBe(false);
    // Then nothing: a server half restarted under a socket with no hello.
    expect(s.needsObserver(115_000 + LIVE_STALE_MS)).toBe(true);
    expect(s.needsObserver(115_000 + LIVE_STALE_MS + 2_000)).toBe(false);
    expect(s.needsObserver(115_000 + 2 * LIVE_STALE_MS)).toBe(true);
    // Its `watching: false` closes it, and a closed picture is owed nothing.
    s.applyObserver(cleared, 8, 150_000);
    expect(s.needsObserver(200_000)).toBe(false);
  });

  it("a re-hydrate keeps the picture, drops the ground and owes one `arena.observer` ask", () => {
    const s = new ReplayStore();
    s.applyObserver(live(), 5, 0);
    s.applyMap(ground());
    s.clearForRehydrate();
    expect(s.open && s.live).toBe(true);
    expect(s.doc).not.toBeNull();
    expect(s.map).toBeNull();
    expect(s.needsObserver(0)).toBe(true);
    expect(s.needsDocument(0)).toBe(false);
  });
});
