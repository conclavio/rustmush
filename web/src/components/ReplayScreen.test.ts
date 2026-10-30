// @vitest-environment jsdom
//
// ReplayScreen mounted (16b cut I.2; 16c N9, Q37): the committed replay of
// match 7 on its ground, both sides by callsign, the header saying so, the
// label saying when the canvas is drawing between keyframes — and nothing on
// the screen that the document did not carry. The app handed to it is the
// replay store plus DECOYS in the stores a careless screen might join a unit
// to a name through (who, presence, the unit feed): a name and an object number
// that must never appear.
//
// jsdom has no 2D canvas; `getContext` answers a context whose every drawing
// call is a no-op spy (the cockpit's test pins the paint itself).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import ReplayScreen from "./ReplayScreen.svelte";
import type { GameApp } from "$lib/game.svelte";
import { WAITING_FOR_THE_REPLAY } from "$lib/replay";
import { ReplayStore } from "$lib/stores/replay.svelte";

const read = (dir: string, name: string): Record<string, unknown> =>
  JSON.parse(readFileSync(resolve(process.cwd(), `src/lib/fixtures/${dir}`, name), "utf8")) as Record<
    string,
    unknown
  >;

let original: typeof HTMLCanvasElement.prototype.getContext;

beforeEach(() => {
  original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function () {
    const ctx: Record<string, unknown> = { fillStyle: "", strokeStyle: "", font: "", lineWidth: 1 };
    for (const op of [
      "fillRect",
      "fillText",
      "strokeText",
      "arc",
      "beginPath",
      "fill",
      "stroke",
      "clearRect",
      "drawImage",
      "strokeRect",
    ]) {
      ctx[op] = vi.fn();
    }
    return ctx;
  } as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = original;
});

/** Mount on a replay store with the decoys beside it. */
function mount(): { replay: ReplayStore } {
  const replay = new ReplayStore();
  const app = {
    replay,
    who: { entries: [{ name: "Decoy Pilot", id: "#909" }] },
    presence: { thinking: [{ name: "Decoy Pilot" }] },
    unit: { view: { object: { id: "#909", name: "Decoy Tank" } } },
  } as unknown as GameApp;
  render(ReplayScreen, { props: { app } });
  return { replay };
}

async function opened(): Promise<{ replay: ReplayStore }> {
  const m = mount();
  m.replay.applyGrid(read("replay", "replay_window.json"), 5);
  m.replay.applyDocument({
    schema: { id: "arena.replay", version: 1, shape: "replay" },
    ...read("replay", "replay.json"),
  });
  m.replay.applyMap(read("cockpit", "arena_map.json"));
  await tick();
  return m;
}

describe("ReplayScreen mounted", () => {
  it("waits for the document before anything", async () => {
    const { replay } = mount();
    replay.applyGrid(read("replay", "replay_window.json"), 5);
    await tick();
    expect(screen.getByText(WAITING_FOR_THE_REPLAY)).toBeTruthy();
    expect(document.querySelector("canvas")).toBeNull();
  });

  it("shows both sides by callsign, says so, and names nothing the document does not carry", async () => {
    await opened();
    const body = document.body.textContent ?? "";
    expect(body).toContain(
      "Replay of match 7 — aborted — both sides are shown: this match is over.",
    );
    const units = [...document.querySelectorAll("button.unit")].map((b) => b.textContent);
    expect(units).toEqual(["RA", "RB", "BA", "BB"]);
    expect(body).toContain("Keyframe 2 of 3 (0:05): the arena's own positions.");
    expect(document.querySelector("canvas")).not.toBeNull();
    const range = document.querySelector('input[type="range"]') as HTMLInputElement;
    expect(range.max).toBe("10000");
    expect(range.step).toBe("100");
    for (const decoy of ["Decoy", "#909", "Decoy Tank"]) {
      expect(body).not.toContain(decoy);
    }
    expect(body).not.toMatch(/#\d/);
  });

  it("says it is drawing between keyframes off a keyframe, and lists what happened by then", async () => {
    const { replay } = await opened();
    const range = document.querySelector('input[type="range"]') as HTMLInputElement;
    await fireEvent.input(range, { target: { value: "7500" } });
    await tick();
    expect(replay.cursorMs).toBe(7500);
    const body = document.body.textContent ?? "";
    expect(body).toContain("Between keyframes 2 and 3: drawn, not recorded.");
    expect(body).toContain("0:06  RA at BA: hit");
    expect(body).toContain("0:07  BB wrecked");
    expect(body).not.toContain("0:08  BA at RA: miss");
  });

  it("moves the focus from a unit's button, and plays to the end at 1×", async () => {
    vi.useFakeTimers();
    try {
      const { replay } = await opened();
      const units = [...document.querySelectorAll("button.unit")];
      await fireEvent.click(units[2]!);
      expect(replay.focus).toBe(2);
      await fireEvent.click(screen.getByRole("button", { name: "play" }));
      expect(replay.playing).toBe(true);
      vi.advanceTimersByTime(1_000);
      expect(replay.cursorMs).toBe(6000);
      vi.advanceTimersByTime(10_000);
      expect(replay.cursorMs).toBe(10000);
      expect(replay.playing).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("cycles between overview, tactical, and cockpit views via v key and buttons", async () => {
    await opened();
    const viewBtns = [...document.querySelectorAll("button.view-btn")];
    expect(viewBtns.map((b) => b.textContent)).toEqual(["overview", "tactical", "cockpit"]);
    expect(viewBtns[0]?.classList.contains("active")).toBe(true);

    await fireEvent.keyDown(window, { key: "v" });
    await tick();
    expect(viewBtns[1]?.classList.contains("active")).toBe(true);

    await fireEvent.keyDown(window, { key: "v" });
    await tick();
    expect(viewBtns[2]?.classList.contains("active")).toBe(true);

    await fireEvent.click(viewBtns[0]!);
    await tick();
    expect(viewBtns[0]?.classList.contains("active")).toBe(true);
  });
});

// The live picture (16b cut I.4; 16c N10): the same screen in LIVE mode — no
// scrubber and no play, the label that says what it is, a status pane in
// place of the shots and the wrecks, and the object numbers the picture
// carries (a staff surface).
describe("ReplayScreen mounted live", () => {
  it("draws the picture with no scrubber, says it is live, and names each unit's number", async () => {
    const { replay } = mount();
    replay.applyObserver(
      {
        schema: { id: "arena.observer", version: 1, retain: "latest" },
        match: 7,
        now_ms: 90_000,
        tempo: "engaged",
        floor_ms: null,
        cell_cm: 1_000,
        width: 96,
        height: 64,
        in_flight: 2,
        units: [
          ["RA", "R", 6, 4, 90, 301, "pm"],
          ["BA", "B", 20, 14, 270, 302, "ph"],
        ],
      },
      5,
      0,
    );
    replay.applyMap(read("cockpit", "arena_map.json"));
    await tick();
    const body = document.body.textContent ?? "";
    expect(body).toContain("Watching 7 — 1:30 (live)");
    expect(body).toContain("Live: the arena's own positions, at most once an arena second.");
    expect(document.querySelector('input[type="range"]')).toBeNull();
    expect(screen.queryByRole("button", { name: "play" })).toBeNull();
    expect(document.querySelector("canvas")).not.toBeNull();
    for (const line of ["Match 7  engaged", "Live, at most once a second.", "In flight: 2", "@arena/watch off stops it."]) {
      expect(body).toContain(line);
    }
    expect(body).toContain("#301  machine");
    expect(body).toContain("#302  pilot  held");
    expect(body).not.toContain("No shots yet.");
    // Still nothing from the decoys beside it.
    expect(body).not.toContain("Decoy");
    expect(body).not.toContain("#909");
  });
});
