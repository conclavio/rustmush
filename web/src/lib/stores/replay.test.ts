// ReplayStore (16b cut I.2): the one-shot document ask, the map latch, the
// dismissal keyed on the envelope seq, `available: false` closing the picture,
// and the re-hydrate that keeps the picture open and asks again.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { MAP_ASK_MS } from "./cockpit.svelte";
import { REPLAY_ASK_MS, ReplayStore } from "./replay.svelte";

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
