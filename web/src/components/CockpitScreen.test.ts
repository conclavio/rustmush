// @vitest-environment jsdom
//
// CockpitScreen mounted (16b cut B.2, `T-44`): the first component this suite
// renders into a DOM. The rest of the suite stays in node — this file alone
// opts into jsdom on its first line — because the stores and the controller
// need none, and a DOM for all of them would only hide which tests need one.
//
// **The rig**, for any later mount of this screen (16b cut B.3's seated-gate
// test reuses it): the app is a `CockpitStore` and a `focusComposer` spy and
// nothing else, which is also the whole of what the screen may reach for (the
// whitelist probe in `game.test.ts`); jsdom has no 2D canvas, so
// `HTMLCanvasElement.prototype.getContext` answers a RECORDING context of that
// canvas's own — the screen's and the offscreen terrain cache's apart — and
// every call lands in ONE log, in call order, with the canvas it was drawn on
// and the fill and stroke styles it was made under. The frame and the map are
// the committed fixtures every renderer reads (`fixtures/cockpit/`), so the
// geometry below is the fixture's: a 96 by 64 ground, `@` RA at (6,4), `x BA`
// at (36,14) and locked, `+ RB` at (2,9), `? X3` at (14,7), `? X4` at (50,33) —
// and (16b cut V.1) the named spots `A` The Ford at (25,3), `B` The Roof at
// (14,7) under X3, and `C` The Woods at (50,25).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CockpitScreen from "./CockpitScreen.svelte";
import { NOT_IN_A_COCKPIT } from "$lib/cockpit";
import type { GameApp } from "$lib/game.svelte";
import { CockpitStore } from "$lib/stores/cockpit.svelte";

const DIR = "src/lib/fixtures/cockpit";

function json(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(resolve(process.cwd(), DIR, name), "utf8")) as Record<
    string,
    unknown
  >;
}

/** One logged call on a recording context. */
interface Call {
  op: string;
  args: unknown[];
  /** The canvas whose context took the call. */
  canvas: HTMLCanvasElement;
  fillStyle: string;
  strokeStyle: string;
}

/** The recording rig: one 2D context per canvas, made on its first
 * `getContext`, and one shared `log` in call order. */
function recordingRig(): {
  contextFor: (canvas: HTMLCanvasElement) => Record<string, unknown>;
  log: Call[];
} {
  const log: Call[] = [];
  const contexts = new Map<HTMLCanvasElement, Record<string, unknown>>();
  return {
    log,
    contextFor: (canvas) => {
      let ctx = contexts.get(canvas);
      if (ctx === undefined) {
        ctx = recordingContext(canvas, log);
        contexts.set(canvas, ctx);
      }
      return ctx;
    },
  };
}

/** A recording 2D context for `canvas`: every drawing call as a `vi.fn`, the
 * style properties settable, and each call appended to `log` with its canvas
 * and the styles it was made under — so a colour is asserted where it was
 * USED, not where it was set, and a call is asserted on the canvas it hit. */
function recordingContext(canvas: HTMLCanvasElement, log: Call[]): Record<string, unknown> {
  const ctx: Record<string, unknown> = {
    fillStyle: "",
    strokeStyle: "",
    font: "",
    lineWidth: 1,
    textBaseline: "alphabetic",
  };
  for (const op of [
    "fillRect",
    "fillText",
    "strokeText",
    "arc",
    "beginPath",
    "fill",
    "stroke",
    "moveTo",
    "lineTo",
    "closePath",
    "clearRect",
    "drawImage",
    // 16b cut V.1: a spot's hollow square.
    "strokeRect",
  ]) {
    ctx[op] = vi.fn((...args: unknown[]) => {
      log.push({
        op,
        args,
        canvas,
        fillStyle: String(ctx.fillStyle),
        strokeStyle: String(ctx.strokeStyle),
      });
    });
  }
  return ctx;
}

let fake: ReturnType<typeof recordingRig>;
let original: typeof HTMLCanvasElement.prototype.getContext;

beforeEach(() => {
  fake = recordingRig();
  original = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
    return fake.contextFor(this);
  } as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

afterEach(() => {
  HTMLCanvasElement.prototype.getContext = original;
});

/** Mount the screen on a bare cockpit store; nothing fed yet. */
function mount(): { app: GameApp; cockpit: CockpitStore; focusComposer: ReturnType<typeof vi.fn> } {
  const cockpit = new CockpitStore();
  const focusComposer = vi.fn();
  const app = { cockpit, focusComposer } as unknown as GameApp;
  render(CockpitScreen, { props: { app } });
  return { app, cockpit, focusComposer };
}

/** Mount and feed the fixture frame and its ground. */
async function seated(): Promise<ReturnType<typeof mount>> {
  const m = mount();
  m.cockpit.applyFrame(json("cockpit_frame.json"));
  m.cockpit.applyMap(json("arena_map.json"));
  await tick();
  return m;
}

/** The logged calls of one kind. */
function calls(op: string): Call[] {
  return fake.log.filter((c) => c.op === op);
}

/** Forget what has been drawn so far, so the next assertion counts one paint. */
function resetLog(): void {
  fake.log.length = 0;
}

/** The first argument of every `fillText`/`strokeText`: the labels painted. */
function texts(op: "fillText" | "strokeText"): unknown[] {
  return calls(op).map((c) => c.args[0]);
}

function zoomButton(label: "zoom in" | "zoom out"): HTMLElement {
  return screen.getByRole("button", { name: label });
}

describe("CockpitScreen mounted", () => {
  it("reads `Not in a cockpit.` before any frame", () => {
    mount();
    expect(screen.getByText(NOT_IN_A_COCKPIT)).toBeTruthy();
    expect(document.querySelector("canvas")).toBeNull();
  });

  it("shows the five panes and the legend once seated", async () => {
    await seated();
    const headings = [...document.querySelectorAll("h4")].map((h) => h.textContent);
    expect(headings).toEqual(["status", "damage", "weapons", "contacts", "notices"]);
    expect(document.querySelector(".legendline")?.textContent).toBe(
      "x BA  + RB  ? X3  ? X4  A The Ford  C The Woods",
    );
    expect(screen.queryByText(NOT_IN_A_COCKPIT)).toBeNull();
  });

  it("puts `lock` and `scan` in the box from a contact, and sends nothing", async () => {
    const { focusComposer } = await seated();
    const desigs = [...document.querySelectorAll("button.desig")];
    expect(desigs.map((b) => b.textContent)).toEqual(["BA", "RB", "X3", "X4"]);
    await fireEvent.click(desigs[0]!);
    expect(focusComposer).toHaveBeenLastCalledWith("lock BA");
    const scans = [...document.querySelectorAll("button.scanbtn")];
    expect(scans).toHaveLength(4);
    await fireEvent.click(scans[0]!);
    expect(focusComposer).toHaveBeenLastCalledWith("scan BA");
    expect(focusComposer).toHaveBeenCalledTimes(2);
  });

  it("bands BA's chips from its reading", async () => {
    await seated();
    // BA is the one contact with a close look on it (`ggyrd` over the one table).
    const chips = [...document.querySelectorAll(".chips")];
    expect(chips).toHaveLength(1);
    const bands = [...chips[0]!.querySelectorAll(".chip")].map((c) => [
      c.textContent,
      c.getAttribute("data-band"),
    ]);
    expect(bands).toEqual([
      ["engine", "green"],
      ["hull", "green"],
      ["left_track", "yellow"],
      ["right_track", "red"],
      ["turret", "destroyed"],
    ]);
  });

  it("zooms in with `+` and out with `−`, and the readout follows", async () => {
    await seated();
    const zin = zoomButton("zoom in");
    const zout = zoomButton("zoom out");
    expect(zin.textContent).toBe("+");
    expect(zout.textContent).toBe("−");
    expect(screen.getByText("128 cells")).toBeTruthy();
    await fireEvent.click(zin);
    expect(screen.getByText("64 cells")).toBeTruthy();
    await fireEvent.click(zout); // back to 128
    await fireEvent.click(zout);
    expect(screen.getByText("256 cells")).toBeTruthy();
  });

  it("draws on a canvas named for the screen reader", async () => {
    await seated();
    const canvas = document.querySelector("canvas");
    expect(canvas?.getAttribute("aria-label")).toBe("arena window");
    // The whole 96 by 64 ground at 5 px a cell (`floor(560 / 96)`).
    expect(canvas?.width).toBe(96 * 5);
    expect(canvas?.height).toBe(64 * 5);
  });

  it("colours each marker by its kind, wedges `@`, and rings the lock", async () => {
    await seated();
    // One `arc` per contact dot, FILLED in its kind's colour, in frame order;
    // the lock's ring is the one further `arc`, STROKED.
    const arcsThen = (next: string): Call[] =>
      fake.log.filter((c, i) => c.op === "arc" && fake.log[i + 1]?.op === next);
    const dots = arcsThen("fill");
    expect(dots.map((c) => c.fillStyle)).toEqual(["#e05a5a", "#4fa86a", "#b9a23a", "#b9a23a"]);
    // `@` is a wedge: one `moveTo` and two `lineTo`s, filled in its own colour.
    expect(calls("moveTo")).toHaveLength(1);
    expect(calls("lineTo")).toHaveLength(2);
    const wedgeFill = fake.log.findIndex((c) => c.op === "closePath");
    expect(fake.log[wedgeFill + 1]).toMatchObject({ op: "fill", fillStyle: "#f2c94c" });
    // The ring: stroked once, in its own colour, around BA.
    const strokes = calls("stroke");
    expect(strokes).toHaveLength(1);
    expect(strokes[0]?.strokeStyle).toBe("#fff8dc");
    const rings = arcsThen("stroke");
    expect(rings).toHaveLength(1);
    expect(rings[0]?.strokeStyle).toBe("#fff8dc");
    expect(rings[0]?.args.slice(0, 2)).toEqual(dots[0]?.args.slice(0, 2)); // BA's centre
  });

  it("labels the lock alone at 128 cells (5 px), and every marker at 64 (8 px)", async () => {
    await seated();
    // At 5 px a hostile and the lock are labelled and nothing else; BA is
    // both, so it alone, over a one-pixel halo (16b cut B.1).
    expect(texts("strokeText")).toEqual(["BA"]);
    expect(texts("fillText")).toEqual(["BA"]);
    // Zoomed in, the window clamps to the origin (x 0..63, y 0..63): every
    // marker is inside it, the cell is 8 px, and all five are labelled with
    // no halo — your own callsign first, then the contacts in frame order.
    resetLog();
    await fireEvent.click(zoomButton("zoom in"));
    const canvas = document.querySelector("canvas");
    expect(canvas?.width).toBe(64 * 8);
    // 16b cut V.1: then the spots that own their cell, LAST, in authored order
    // — `B` The Roof is under X3 and is labelled nowhere.
    expect(texts("fillText")).toEqual(["RA", "BA", "RB", "X3", "X4", "The Ford", "The Woods"]);
    expect(texts("strokeText")).toEqual([]);
    expect(document.querySelector(".legendline")?.textContent).toBe(
      "x BA  + RB  ? X3  ? X4  A The Ford  C The Woods",
    );
  });

  it("squares each spot that owns its cell, in its own colour, and no other (16b cut V.1)", async () => {
    await seated();
    // At 128 cells (the whole ground, 5 px): A and C own their cells, B is under
    // X3 — two squares, stroked on the screen, in the sand tone.
    const squares = (): Call[] => calls("strokeRect");
    expect(squares()).toHaveLength(2);
    const onScreen = document.querySelector("canvas");
    expect(squares().every((c) => c.canvas === onScreen)).toBe(true);
    expect(squares().every((c) => c.strokeStyle === "#d8c8a0")).toBe(true);
    // A at (25, 3): its centre is (127.5, 302.5) north-up at 5 px, and the side
    // is `max(4, 5 * 0.9)` = 4.5.
    expect(squares()[0]?.args).toEqual([125.25, 300.25, 4.5, 4.5]);
    // Not labelled below 6 px, so no spot name is painted.
    expect(texts("fillText")).toEqual(["BA"]);
    // A square is not a path: the lock's ring is still the one `stroke`, and a
    // spot adds no dot.
    expect(calls("stroke")).toHaveLength(1);
    expect(calls("arc")).toHaveLength(5);
    // Zoomed in (64 cells, 8 px): the same two, and now their names.
    resetLog();
    await fireEvent.click(zoomButton("zoom in"));
    expect(squares()).toHaveLength(2);
    expect(squares()[0]?.args).toEqual([25 * 8 + 4 - 3.6, (63 - 3) * 8 + 4 - 3.6, 7.2, 7.2]);
    expect(texts("fillText")).toContain("The Ford");
    expect(texts("fillText")).toContain("The Woods");
    expect(texts("fillText")).not.toContain("The Roof");
  });

  it("paints the terrain once per window: a frame after it costs one drawImage (T-33)", async () => {
    const { cockpit } = mount();
    cockpit.applyFrame(json("cockpit_frame.json"));
    cockpit.applyMap(json("arena_map.json"));
    await tick();
    // The first paint: every cell of the 96 by 64 window, once, into the cache,
    // and the cache copied onto the screen once.
    expect(calls("fillRect")).toHaveLength(96 * 64);
    expect(calls("drawImage")).toHaveLength(1);
    // The ground went into the CACHE, not onto the screen: every cell on one
    // canvas that is not the one in the document …
    const onScreen = document.querySelector("canvas");
    expect(onScreen).not.toBeNull();
    const cache = calls("fillRect")[0]!.canvas;
    expect(cache).not.toBe(onScreen);
    expect(calls("fillRect").every((c) => c.canvas === cache)).toBe(true);
    // … and it is that cache the screen copies, at its corner, BEFORE any
    // marker is drawn — so the ground is under the markers, never over them.
    const copyAt = (): number => fake.log.findIndex((c) => c.op === "drawImage");
    const firstMarker = (): number =>
      fake.log.findIndex((c) => (c.op === "arc" || c.op === "moveTo") && c.canvas === onScreen);
    // Identity, not structural equality: a canvas copied onto ITSELF must fail.
    const copiedCache = (): void => {
      const copy = calls("drawImage")[0]!;
      expect(copy.canvas).toBe(onScreen);
      expect(copy.args[0]).toBe(cache);
      expect(copy.args.slice(1)).toEqual([0, 0]);
    };
    copiedCache();
    expect(firstMarker()).toBeGreaterThan(copyAt());
    expect(calls("arc").every((c) => c.canvas === onScreen)).toBe(true);

    // The next pulse, the same window: the markers again, the ground not.
    resetLog();
    cockpit.applyFrame(json("cockpit_frame.json"));
    await tick();
    expect(calls("fillRect")).toHaveLength(0);
    expect(calls("drawImage")).toHaveLength(1);
    copiedCache();
    expect(calls("arc").length).toBeGreaterThan(0);
    expect(firstMarker()).toBeGreaterThan(copyAt());

    // A new window repaints: zoomed in, 64 by 64 at 8 px.
    resetLog();
    await fireEvent.click(zoomButton("zoom in"));
    expect(calls("fillRect")).toHaveLength(64 * 64);
    expect(calls("drawImage")).toHaveLength(1);

    // Zooming out to 256 on this ground is the same window as 128 — the whole
    // 96 by 64 at 5 px — so the step to 128 repaints and the step to 256 does not.
    resetLog();
    await fireEvent.click(zoomButton("zoom out"));
    expect(calls("fillRect")).toHaveLength(96 * 64);
    resetLog();
    await fireEvent.click(zoomButton("zoom out"));
    expect(screen.getByText("256 cells")).toBeTruthy();
    expect(calls("fillRect")).toHaveLength(0);
    expect(calls("drawImage")).toHaveLength(1);

    // A new map object is new ground, whatever the window: repainted.
    resetLog();
    cockpit.applyMap(json("arena_map.json"));
    await tick();
    expect(calls("fillRect")).toHaveLength(96 * 64);
    expect(calls("drawImage")).toHaveLength(1);
  });
});

describe("CockpitScreen, 16b cut Y.4", () => {
  it("shows a reclaim's catch-up block in the notices pane, before the frame's notices", async () => {
    const m = mount();
    const cases = JSON.parse(
      readFileSync(resolve(process.cwd(), DIR, "catchup_lines.json"), "utf8"),
    ) as { catchup: unknown; lines: string[] }[];
    m.cockpit.applyFrame({ ...json("cockpit_frame.json"), catchup: cases[0]!.catchup });
    m.cockpit.applyMap(json("arena_map.json"));
    await tick();
    const pane = document.querySelector('section[aria-label="notices"]');
    const lines = [...(pane?.querySelectorAll("p.line") ?? [])].map((p) => p.textContent);
    expect(lines[0]).toBe("Back aboard RA after 2 m 14 s away.");
    expect(lines).toEqual([...cases[0]!.lines, "X4 new contact", "BA section destroyed: turret"]);
  });
});

// **17b cut M5.5** (the M5 design note's M5.5 item 2; C-6): the mind's four
// lines drawn as the weapons section's tail — the same placement as the TUI's,
// so the headings stay five — and a click on a sealed profile the mind is not
// flying puts `MIND PROFILE <name>` in the box and sends nothing until the
// pilot presses Enter.
describe("CockpitScreen, 17b cut M5.5", () => {
  const TAIL = [
    "mind Standard Doctrine · Class I",
    "> Sniper   Line",
    "dormant while you fly",
    "you: helm sensors fire",
  ];

  /** The text of every line the weapons section prints, in order. */
  function weaponsSection(): string[] {
    const pane = document.querySelector('section[aria-label="weapons"]');
    return [...(pane?.querySelectorAll("p.line") ?? [])].map((p) => p.textContent ?? "");
  }

  it("draws the mind's four lines as the weapons section's tail", async () => {
    await seated();
    expect(weaponsSection()).toEqual([
      "rules: S1 fire; MIND RULES",
      "cannon  ready  14/20  62%",
      "machine_gun  1 s  380/500  21%",
      ...TAIL,
    ]);
    // No section is added: the headings are the five they were.
    const headings = [...document.querySelectorAll("h4")].map((h) => h.textContent);
    expect(headings).toEqual(["status", "damage", "weapons", "contacts", "notices"]);
    // The profile row: a button for each profile the mind is not flying, the
    // flown one plain text (a switch to it is refused: the review's D4), and
    // the row's text the view model's own line.
    const row = document.querySelector("p.profiles");
    expect(row?.textContent).toBe("> Sniper   Line");
    expect([...(row?.querySelectorAll("span.flown") ?? [])].map((s) => s.textContent)).toEqual([
      "Sniper",
    ]);
    const buttons = [...document.querySelectorAll("button.profile")];
    expect(buttons.map((b) => b.textContent)).toEqual(["Line"]);
    // Each says what its click puts in the box (17b cut M5.5's review).
    expect(buttons.map((b) => b.getAttribute("title"))).toEqual(["MIND PROFILE Line"]);
  });

  it("puts `MIND PROFILE <name>` in the box from a profile, and sends nothing", async () => {
    const { focusComposer } = await seated();
    const buttons = [...document.querySelectorAll("button.profile")];
    await fireEvent.click(buttons[0]!);
    expect(focusComposer).toHaveBeenLastCalledWith("MIND PROFILE Line");
    // The flown profile offers nothing: a click on it fills no box.
    await fireEvent.click(document.querySelector("p.profiles span.flown")!);
    // The composer is filled and nothing else: the screen reaches for no other
    // method of the app, so no line is sent until Enter.
    expect(focusComposer).toHaveBeenCalledTimes(1);
  });

  it("m5_the_mind_tail_takes_the_weapons_place_on_an_unarmed_hull", async () => {
    // The drone: no weapons key, no contact odds, the unchosen slot's seal.
    const m = mount();
    const drone = json("cockpit_frame.json");
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
    m.cockpit.applyFrame(drone);
    m.cockpit.applyMap(json("arena_map.json"));
    await tick();
    expect(weaponsSection()).toEqual([
      "mind Standard Doctrine · Class I",
      "> Line",
      "dormant while you fly",
      "you: helm sensors",
    ]);
    expect(screen.queryByText("(no weapon)")).toBeNull();
    const headings = [...document.querySelectorAll("h4")].map((h) => h.textContent);
    expect(headings).toEqual(["status", "damage", "weapons", "contacts", "notices"]);
    // A set of one offers no switch (line mode: `no other is sealed to switch
    // to`): its one profile is the flown one, plain text, and no button.
    expect(document.querySelector("p.profiles span.flown")?.textContent).toBe("Line");
    expect(document.querySelector("button.profile")).toBeNull();
  });
});
