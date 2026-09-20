// CockpitStore (16b cut Q.5): the notices ring, the pulled map's 2 s latch,
// the once-per-match screen switch, and what a resume has to restore from one
// replayed frame.
//
// The clock is a parameter (`needsMap(now)`), so every latch case here is a
// plain number and there are no fake timers.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { CockpitStore, MAP_ASK_MS } from "./cockpit.svelte";
import { COCKPIT_NOTICES } from "../cockpit";

const DIR = "src/lib/fixtures/cockpit";

function json(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(process.cwd(), DIR, name), "utf8")) as Record<
    string,
    unknown
  >;
}

const FRAME = (): Record<string, unknown> => json("cockpit_frame.json");
const MAP = (): Record<string, unknown> => json("arena_map.json");

/** The cleared frame the bridge sends when a pilot is unbound. */
const CLEARED = {
  schema: { id: "arena.cockpit", version: 1, retain: "latest" },
  seated: false,
};

/** The map query's answer to anyone not seated on a deployed unit. */
const MAP_UNSEATED = {
  schema: { id: "arena.map", version: 1, shape: "terrain", retain: "latest" },
  seated: false,
};

describe("CockpitStore", () => {
  it("holds a seated frame and its ground, and opens the screen once a match", () => {
    const s = new CockpitStore();
    expect(s.seated).toBe(false);
    expect(s.matchId).toBeNull();
    expect(s.applyFrame(FRAME())).toBe(true); // the first frame of match 7
    expect(s.seated).toBe(true);
    expect(s.matchId).toBe(7);
    expect(s.holdsTheGround).toBe(false);
    expect(s.applyFrame(FRAME())).toBe(false); // and never again for match 7
    s.applyMap(MAP());
    expect(s.holdsTheGround).toBe(true);
    expect(s.map?.width).toBe(96);
    // A new match opens the screen again, and drops the old ground with it.
    expect(s.applyFrame({ ...FRAME(), match: 8 })).toBe(true);
    expect(s.map).toBeNull();
    expect(s.holdsTheGround).toBe(false);
  });

  it("rings a notice from kind + designator, never from its text", () => {
    const s = new CockpitStore();
    s.applyFrame(FRAME());
    expect(s.notices).toEqual(["X4 new contact", "BA section destroyed: turret"]);
    // The server's own sentences are in the payload and in NONE of the ring.
    const flat = s.notices.join("\n");
    expect(flat).not.toContain("New contact X4: bearing 030");
    expect(flat).not.toContain("BA's turret is destroyed.");
    // The ring keeps the last fifty and nothing more.
    for (let i = 0; i < 60; i += 1) {
      s.applyFrame({
        ...FRAME(),
        notices: [{ kind: "hit", contact: null, designator: "BA", section: null, text: "x" }],
      });
    }
    expect(s.notices).toHaveLength(COCKPIT_NOTICES);
    expect(s.notices[COCKPIT_NOTICES - 1]).toBe("BA hit");
    // A frame with no notices rings nothing (the mirror a read verb re-emits
    // is stored with them cleared).
    s.applyFrame({ ...FRAME(), notices: [] });
    expect(s.notices).toHaveLength(COCKPIT_NOTICES);
  });

  it("asks for the map at once, not again inside 2 s, and again after", () => {
    const s = new CockpitStore();
    expect(s.needsMap(1000)).toBe(false); // not seated: nothing to draw
    s.applyFrame(FRAME());
    expect(s.needsMap(1000)).toBe(true); // at once
    expect(s.needsMap(1000)).toBe(false);
    expect(s.needsMap(1000 + MAP_ASK_MS - 1)).toBe(false);
    expect(s.needsMap(1000 + MAP_ASK_MS)).toBe(true);
    // Once the ground is held it is never asked for again.
    s.applyMap(MAP());
    expect(s.needsMap(1_000_000)).toBe(false);
    // A fresh match asks on that very frame rather than waiting out a latch
    // armed for the old one.
    s.applyFrame({ ...FRAME(), match: 9 });
    expect(s.needsMap(1000 + MAP_ASK_MS + 1)).toBe(true);
  });

  it("counts the hello pass's ask as the first one (noteMapAsked)", () => {
    // The hello pass pulls the ground before any frame has named a match — it
    // must, because the map is never ring-recorded and a re-attach holds none
    // — and that ask covers whichever match the first frame turns out to be.
    // Without it a pilot re-attaching mid-match was sent the WHOLE arena's
    // terrain twice, on a socket that had only just reopened.
    const s = new CockpitStore();
    s.noteMapAsked(1000);
    s.applyFrame(FRAME());
    expect(s.needsMap(1000)).toBe(false);
    expect(s.needsMap(1000 + MAP_ASK_MS - 1)).toBe(false);
    // It is a wildcard and not a free pass: an ask that is never answered ages
    // out and the self-heal takes over on its own cadence.
    expect(s.needsMap(1000 + MAP_ASK_MS)).toBe(true);
    // Once it is spent it behaves like any other ask: a fresh match asks on
    // that very frame.
    s.applyFrame({ ...FRAME(), match: 11 });
    expect(s.needsMap(1000 + MAP_ASK_MS)).toBe(true);
  });

  it("drops a wrong answer WITHOUT re-arming the latch", () => {
    const s = new CockpitStore();
    s.applyFrame(FRAME());
    expect(s.needsMap(0)).toBe(true);
    // The unseated answer, and ground of another match: both held as nothing.
    s.applyMap(MAP_UNSEATED);
    expect(s.map).toBeNull();
    s.applyMap({ ...MAP(), match: 4 });
    expect(s.holdsTheGround).toBe(false);
    // …and neither bought a fresh ask: the latch still runs out on its own.
    expect(s.needsMap(MAP_ASK_MS - 1)).toBe(false);
    expect(s.needsMap(MAP_ASK_MS)).toBe(true);
  });

  it("clears on an unseated frame and stops asking", () => {
    const s = new CockpitStore();
    s.applyFrame(FRAME());
    s.applyMap(MAP());
    expect(s.notices.length).toBeGreaterThan(0);
    expect(s.applyFrame(CLEARED)).toBe(false);
    expect(s.seated).toBe(false);
    expect(s.frame).toBeNull();
    expect(s.map).toBeNull();
    expect(s.notices).toEqual([]);
    expect(s.needsMap(0)).toBe(false);
    // The next match starts clean: the screen opens itself again.
    expect(s.applyFrame(FRAME())).toBe(true);
  });

  it("comes back whole from one replayed frame and one map (the resume)", () => {
    // `schema.retain == "latest"` leaves exactly ONE cockpit frame in the ring
    // and the map is never recorded at all, so this is the whole of what a
    // resumed client is handed.
    const s = new CockpitStore();
    expect(s.applyFrame(FRAME())).toBe(true);
    expect(s.needsMap(0)).toBe(true); // it must re-ask: it holds no ground
    s.applyMap(MAP());
    expect(s.seated).toBe(true);
    expect(s.matchId).toBe(7);
    expect(s.holdsTheGround).toBe(true);
    expect(s.frame?.contacts).toHaveLength(4);
    expect(s.notices).toHaveLength(2);
  });

  it("ignores a payload that is not a frame at all", () => {
    const s = new CockpitStore();
    expect(s.applyFrame("nope")).toBe(false);
    expect(s.seated).toBe(false);
    s.applyMap(42);
    expect(s.map).toBeNull();
  });
});
