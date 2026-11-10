// @vitest-environment jsdom
//
// PictureView mounted (doc 21a `X-02`): what the arrow keys hold is let go
// when their release can no longer be seen. The app is the picture's store
// and the one spy the view may reach for (`helm`). Fake timers drive the
// helm's repeats, the throttle's gap and the look's steps.
//
// And (doc 21a `X-04`) the picture drawn here, with the engine's module
// faked — a frame that never changes — and animation frames as timers: a
// fresh canvas gets that frame whether or not the picture changed.

//
// And (doc 21 cut G.12c) §7.3's keys on the committed fixture frame: the
// lines `w`, `s`, `x`, `f`, `g` and `t` send through `helm`, the `F1`
// prefill, the speed's coalescing window, and `v`'s overhead.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import PictureView from "./PictureView.svelte";
import { onlyReaches } from "./reach";
import { decodeCockpitFrame, type CockpitFrame } from "$lib/cockpit";
import type { GameApp } from "$lib/game.svelte";
import { ORDER_GAP_MS } from "$lib/helm";
import {
  LOOK_STEP_MS,
  PICTURE_PREF_KEY,
  SPEED_COALESCE_MS,
  swingLook,
  type PictureKind,
} from "$lib/picture";
import { LOCAL_FRAME_MS } from "$lib/pixel/local";
import {
  KIND_COCKPIT,
  KIND_DOCKING,
  KIND_HOLOTANK,
  KIND_ISO25D,
  KIND_OVERHEAD,
  KIND_SPACE_COCKPIT,
} from "$lib/pixel/wasm";
import { PictureStore } from "$lib/stores/picture.svelte";

/** The engine's module, faked: every render succeeds and hashes by camera kind
 * (`KIND_COCKPIT` vs `KIND_OVERHEAD` vs `KIND_ISO25D`), as an unchanged scene does. */
const fake = vi.hoisted(() => {
  let kind = 1;
  return {
    resetKind: () => {
      kind = 1;
    },
    module: {
      setKind: (k: number) => {
        kind = k;
        return true;
      },
      feed: () => true,
      render: () => true,
      hash: () =>
        kind === 2
          ? "overhead frame"
          : kind === 6
            ? "iso25d frame"
            : kind === 7
              ? "holotank frame"
              : kind === 8
                ? "space cockpit frame"
                : kind === 9
                  ? "docking frame"
                  : "cockpit frame",
      image: () => ({ width: 1, height: 1, data: new Uint8ClampedArray(4) }),
      pixels: () => new Uint8Array(0),
      reset: () => {},
    },
  };
});

vi.mock("$lib/pixel/local", async (importOriginal) => {
  const orig = await importOriginal<typeof import("$lib/pixel/local")>();
  return {
    ...orig,
    localCapable: () => true,
    pixelModule: () => Promise.resolve(fake.module),
  };
});

/** The fixture tank's heading, so a tap is `HEADING 44` or `46`, a held turn
 * swings on from 45, and a stop where nothing turned is `HEADING 45`. */
const HEADING = 45;

function mount(cockpit: CockpitFrame | null = null): {
  picture: PictureStore;
  helm: ReturnType<typeof vi.fn>;
  prefill: ReturnType<typeof vi.fn>;
  frame: HTMLElement;
  unmount: () => void;
} {
  const picture = new PictureStore();
  const helm = vi.fn();
  const prefill = vi.fn();
  // Doc 21 cut G.3c: the picture reads its own store and orders through
  // `helm`, the one send the cockpit tab makes; nothing else (no `submit`; the
  // gateway is asked for no picture, doc 21 cut G.8). Every mount enforces it. `chart`
  // too, read-only: SF.18's space views draw the sky chart's frame behind
  // the ship, reached through a cast (`(app as { chart?… }).chart`) that the
  // source probe this replaces could not see (doc 43 cut TS.19).
  const { app } = onlyReaches({ picture, helm } as unknown as GameApp, [
    "picture",
    "helm",
    "chart",
  ]);
  const { unmount } = render(PictureView, {
    props: { app, headingDeg: HEADING, frame: cockpit, prefill },
  });
  return { picture, helm, prefill, frame: screen.getByRole("application"), unmount };
}

/** The committed fixture frame (BA locked; contacts BA hostile 450 m, RB
 * friendly 100 m, X3 and X4 unidentified at 200 and 600 m), at rest, with
 * G.12a's keys as given. */
function fixtureFrame(over: Record<string, unknown> = {}): CockpitFrame {
  const raw = JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/cockpit_frame.json"), "utf8"),
  ) as Record<string, unknown>;
  return decodeCockpitFrame({ ...raw, speed_kph: 0, ...over })!;
}

/** The page's visibility, as the view reads it, and the event that says it
 * changed. */
function setVisibility(state: "visible" | "hidden"): void {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => state });
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  vi.useFakeTimers();
  fake.resetKind();
  localStorage.setItem(PICTURE_PREF_KEY, "on");
});

afterEach(() => {
  // The stubs first (they were put over the fake timers' own globals), the
  // real timers after, so nothing of the fake clock is left behind.
  vi.unstubAllGlobals();
  vi.useRealTimers();
  localStorage.removeItem(PICTURE_PREF_KEY);
  localStorage.removeItem("rustmush.pictureHd");
  delete (document as { visibilityState?: unknown }).visibilityState;
});

describe("PictureView mounted: a held key is let go when its release cannot be seen", () => {
  it("orders the turn while the arrow is down, a little ahead of the picture", async () => {
    const { picture, helm, frame } = mount();
    await tick();
    expect(picture.on).toBe(true);
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    // The press orders the tap's one degree (doc 21 cut G.4b)…
    expect(helm.mock.calls).toEqual([["HEADING 44 QUIET"]]);
    // …and every gap the heading the picture will show a gap on: 2.5° swung
    // and 7.5° ahead, then 10° swung and 7.5° ahead.
    vi.advanceTimersByTime(ORDER_GAP_MS);
    expect(helm.mock.calls).toEqual([["HEADING 44 QUIET"], ["HEADING 35 QUIET"]]);
    vi.advanceTimersByTime(ORDER_GAP_MS);
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 28 QUIET"]);
    // The release orders the heading the picture shows, ten degrees round.
    await fireEvent.keyUp(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 35 QUIET"]);
    vi.advanceTimersByTime(ORDER_GAP_MS * 20);
    expect(helm.mock.calls).toHaveLength(4);
  });

  it("stops the turn when the page goes hidden, and orders nothing more", async () => {
    const { picture, helm, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowRight" });
    vi.advanceTimersByTime(ORDER_GAP_MS * 2);
    expect(helm.mock.calls).toEqual([
      ["HEADING 46 QUIET"],
      ["HEADING 55 QUIET"],
      ["HEADING 63 QUIET"],
    ]);
    setVisibility("hidden");
    await tick();
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 55 QUIET"]);
    // The release of the arrow, out of sight, changes nothing; nor do the
    // orders the hold would have gone on to give.
    vi.advanceTimersByTime(ORDER_GAP_MS * 20);
    await fireEvent.keyUp(frame, { key: "ArrowRight" });
    expect(helm.mock.calls).toHaveLength(4);
    // The picture itself is off while hidden and on again after.
    expect(picture.on).toBe(false);
    setVisibility("visible");
    await tick();
    expect(picture.on).toBe(true);
  });

  it("stops the turn when the window loses focus", async () => {
    const { helm, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    vi.advanceTimersByTime(ORDER_GAP_MS * 2);
    window.dispatchEvent(new Event("blur"));
    await tick();
    expect(helm.mock.calls).toEqual([
      ["HEADING 44 QUIET"],
      ["HEADING 35 QUIET"],
      ["HEADING 28 QUIET"],
      ["HEADING 35 QUIET"],
    ]);
    vi.advanceTimersByTime(ORDER_GAP_MS * 20);
    expect(helm.mock.calls).toHaveLength(4);
  });

  it("lets go when torn down mid-turn, as a blur would", async () => {
    const { helm, frame, unmount } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    vi.advanceTimersByTime(ORDER_GAP_MS);
    expect(helm.mock.calls).toEqual([["HEADING 44 QUIET"], ["HEADING 35 QUIET"]]);
    unmount();
    // 2.5° round when it went: the stop is the nearest whole degree.
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 43 QUIET"]);
    // The helm's clock went with the view.
    vi.advanceTimersByTime(ORDER_GAP_MS * 20);
    expect(helm.mock.calls).toHaveLength(3);
  });

  it("turns one degree for a press let go at once, in the one line", async () => {
    const { helm, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowRight" });
    await fireEvent.keyUp(frame, { key: "ArrowRight" });
    vi.advanceTimersByTime(ORDER_GAP_MS * 20);
    expect(helm.mock.calls).toEqual([["HEADING 46 QUIET"]]);
  });

  it("stops the look's swing when the page goes hidden", async () => {
    const { picture, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "," });
    vi.advanceTimersByTime(LOOK_STEP_MS);
    const step = swingLook(0, -1, LOOK_STEP_MS);
    expect(picture.lookMdeg).toBe(step);
    setVisibility("hidden");
    await tick();
    // The swing stops; where the look stands is the viewer's own and is kept.
    expect(picture.lookMdeg).toBe(step);
    vi.advanceTimersByTime(LOOK_STEP_MS * 5);
    expect(picture.lookMdeg).toBe(step);
  });
});

describe("PictureView mounted: the cockpit's keys (doc 21 §7.3, cut G.12c)", () => {
  /** A key pressed on the picture; `false` when the view took it from the
   * browser (`preventDefault`). */
  async function press(el: HTMLElement, key: string, init: KeyboardEventInit = {}): Promise<boolean> {
    return fireEvent.keyDown(el, { key, ...init });
  }

  it("w from rest orders a tenth of the top speed at once, and shows it", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    expect(await press(frame, "w")).toBe(false);
    // The first step waits for nothing (the TUI's throttle).
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"]]);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 4);
    expect(helm).toHaveBeenCalledTimes(1);
    await tick();
    expect(screen.getByLabelText("speed ordered").textContent).toBe("spd 0→6");
  });

  it("w from 58 kph clamps at the top speed", async () => {
    const { helm, frame } = mount(fixtureFrame({ speed_kph: 58, max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "w");
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
    expect(helm.mock.calls).toEqual([["SPEED 60 QUIET"]]);
  });

  it("steps from the frame's defaults when it predates the ceilings", async () => {
    // G.12a's fixture names the line tank's ceilings; a frame from before them does not.
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: undefined, reverse_kph: undefined }));
    await tick();
    await press(frame, "w");
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"]]);
  });

  it("s from rest goes into reverse a step at a time and floors at the reverse ceiling", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    for (let i = 0; i < 5; i++) {
      await press(frame, "s");
      vi.advanceTimersByTime(SPEED_COALESCE_MS);
    }
    expect(helm.mock.calls).toEqual([
      ["SPEED -6 QUIET"],
      ["SPEED -12 QUIET"],
      ["SPEED -18 QUIET"],
      ["SPEED -20 QUIET"],
      ["SPEED -20 QUIET"],
    ]);
  });

  it("a held w sends its first step at once and the rest a gap apart, the last value", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "w");
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"]]);
    // Two more inside the gap: one line when it closes, carrying both.
    vi.advanceTimersByTime(SPEED_COALESCE_MS / 5);
    await press(frame, "w", { repeat: true });
    vi.advanceTimersByTime(SPEED_COALESCE_MS / 5);
    await press(frame, "w", { repeat: true });
    expect(helm).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"], ["SPEED 18 QUIET"]]);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 4);
    expect(helm).toHaveBeenCalledTimes(2);
  });

  it("x orders SPEED 0 at once, drops a waiting step, and leaves the heading alone", async () => {
    const { helm, frame } = mount(fixtureFrame({ speed_kph: 30, max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "w");
    await press(frame, "w", { repeat: true });
    await press(frame, "x");
    // The second step was still waiting: it is dropped, and no HEADING is sent.
    expect(helm.mock.calls).toEqual([["SPEED 36 QUIET"], ["SPEED 0 QUIET"]]);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 4);
    expect(helm).toHaveBeenCalledTimes(2);
    await tick();
    expect(screen.getByLabelText("speed ordered").textContent).toBe("spd 30→0");
  });

  it("x lets go a held turn where the picture stands", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "d");
    vi.advanceTimersByTime(500);
    await press(frame, "x");
    expect(helm.mock.calls.at(-2)).toEqual(["SPEED 0 QUIET"]);
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 55 QUIET"]);
    const sent = helm.mock.calls.length;
    // The turn is over: the key still down orders nothing more.
    vi.advanceTimersByTime(ORDER_GAP_MS * 8);
    expect(helm).toHaveBeenCalledTimes(sent);
  });

  it("a capital is its own key, never its lower case's alias", async () => {
    const { helm, prefill, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    for (const key of ["W", "S", "A", "D", "X", "G", "Q", "E", "C", "V"]) {
      expect(await press(frame, key, { shiftKey: true }), key).toBe(true);
    }
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 4);
    expect(helm).not.toHaveBeenCalled();
    expect(prefill).not.toHaveBeenCalled();
  });

  it("f under hold orders FIRE FREE, and the header says HOLD", async () => {
    const { helm, frame } = mount(fixtureFrame({ weapons_free: false }));
    await tick();
    expect(screen.getByText("weapons HOLD")).toBeTruthy();
    await press(frame, "f");
    expect(helm.mock.calls).toEqual([["FIRE FREE"]]);
    // The flip is assumed until the next frame: the header follows, and a
    // second press sends the other verb, not the same one twice.
    await tick();
    expect(screen.getByText("weapons free")).toBeTruthy();
    await press(frame, "f");
    expect(helm.mock.calls).toEqual([["FIRE FREE"], ["FIRE HOLD"]]);
  });

  it("n scans the lock, z and P start up, m flips EMCON on a rig with the pod", async () => {
    const { helm, frame } = mount(fixtureFrame({ ecm: "on" }));
    await tick();
    await press(frame, "n");
    await press(frame, "z");
    await press(frame, "P", { shiftKey: true });
    await press(frame, "m");
    await press(frame, "m");
    expect(helm.mock.calls).toEqual([["SCAN BA"], ["STARTUP"], ["STARTUP"], ["EMCON off"], ["EMCON on"]]);
  });

  it("n scans the lock just asked for, and m is nothing without the pod", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "m");
    expect(helm).not.toHaveBeenCalled();
    await press(frame, "t");
    await press(frame, "n");
    expect(helm.mock.calls).toEqual([["LOCK X3"], ["SCAN X3"]]);
  });

  it("l arms the standing lock rule and removes the slot the frame shows, never a guessed one", async () => {
    const bare = mount(fixtureFrame({ rules: [] }));
    await tick();
    await press(bare.frame, "l");
    expect(bare.helm.mock.calls).toEqual([["LOCK nearest WHEN nolock"]]);
    // No frame has shown the rule's slot yet: the disarm names none.
    await press(bare.frame, "l");
    expect(bare.helm).toHaveBeenCalledTimes(1);
    bare.unmount();

    const lockRule = { slot: 3, rule: { action: "lock" }, text: "locks the nearest contact" };
    const held = mount(fixtureFrame({ rules: [lockRule] }));
    await tick();
    await press(held.frame, "l");
    expect(held.helm.mock.calls).toEqual([["MIND RULE 3 OFF"]]);
  });

  it("F removes the fire slots the frame shows, and arms one under an open gate", async () => {
    // The fixture's cannon holds a fire rule in slot 1.
    const held = mount(fixtureFrame());
    await tick();
    await press(held.frame, "F", { shiftKey: true });
    expect(held.helm.mock.calls).toEqual([["MIND RULE 1 OFF"]]);
    held.unmount();

    const bare = mount(fixtureFrame({ rules: [], weapons_free: false }));
    await tick();
    await press(bare.frame, "F", { shiftKey: true });
    expect(bare.helm.mock.calls).toEqual([["FIRE FREE"], ["FIRE ALL WHEN odds >= 60"]]);
    // Disarmed again before a frame gave it a slot: the one disarm that reaches it.
    await press(bare.frame, "F", { shiftKey: true });
    expect(bare.helm.mock.calls.at(-1)).toEqual(["FIRE ALL OFF"]);
  });

  it("y and N answer the detachment's order, and nothing without one", async () => {
    const none = mount(fixtureFrame());
    await tick();
    await press(none.frame, "y");
    await press(none.frame, "N", { shiftKey: true });
    expect(none.helm).not.toHaveBeenCalled();
    none.unmount();

    const detachment = {
      id: "7",
      revision: "2",
      term: "3",
      leader_slot: 1,
      slots: [{ slot: 1, callsign: "RA", pilot: "Aster", controller: "player", order_prefill: "ORDER DT:7:2:3 1 " }],
      all_order_prefill: "ORDER DT:7:2:3 ALL ",
      my_order: { sequence: "9", from: "Aster", objective: "hold", response: null },
      wilco_command: "WILCO DT:7:2 9",
      negative_command: "NEGATIVE DT:7:2 9",
    };
    const { helm, frame } = mount(fixtureFrame({ detachment }));
    await tick();
    await press(frame, "y");
    await press(frame, "N", { shiftKey: true });
    expect(helm.mock.calls).toEqual([["WILCO DT:7:2 9"], ["NEGATIVE DT:7:2 9"]]);
  });

  it("f with the key missing reads free and orders FIRE HOLD", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    expect(screen.getByText("weapons free")).toBeTruthy();
    await press(frame, "f");
    expect(helm.mock.calls).toEqual([["FIRE HOLD"]]);
  });

  it("says RANGE in the header on a range and nothing on a skirmish (16c Q-PT1)", async () => {
    const skirmish = mount(fixtureFrame());
    await tick();
    expect(screen.queryByLabelText("match mode")).toBeNull();
    skirmish.unmount();
    mount(fixtureFrame({ mode: "range" }));
    await tick();
    expect(screen.getByLabelText("match mode").textContent).toBe("RANGE");
  });

  it("g orders the volley", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "g");
    expect(helm.mock.calls).toEqual([["FIRE ALL"]]);
  });

  it("Shift+1..Shift+4 (!, @, #, $) and quick-fire buttons fire individual weapon slots", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "!", { shiftKey: true });
    await press(frame, "@", { shiftKey: true });
    await press(frame, "#", { shiftKey: true });
    expect(helm.mock.calls).toEqual([["FIRE cannon"], ["FIRE machine_gun"]]);
    const slotBtn = screen.getByRole("button", { name: "1:cannon" });
    await fireEvent.click(slotBtn);
    expect(helm.mock.calls.at(-1)).toEqual(["FIRE cannon"]);
  });

  it("supports zoom in all three cameras and click-to-lock in first-person", async () => {
    const { helm, picture, frame } = mount(fixtureFrame());
    await tick();
    // In first-person, zoom floors at 100% and steps up to 200%.
    await press(frame, "-");
    expect(screen.getByTitle("Reset zoom").textContent).toBe("100%");
    await press(frame, "+");
    expect(screen.getByTitle("Reset zoom").textContent).toBe("125%");
    // Each camera keeps its own zoom: the overhead opens at 100% whatever
    // first person was left at, and steps below it.
    picture.kind = "overhead";
    await tick();
    expect(screen.getByTitle("Reset zoom").textContent).toBe("100%");
    await press(frame, "-");
    expect(screen.getByTitle("Reset zoom").textContent).toBe("75%");
    // The 2.5D camera opens zoomed out, and its reset is back to that.
    picture.kind = "iso25d";
    await tick();
    expect(screen.getByTitle("Reset zoom").textContent).toBe("50%");
    await press(frame, "+");
    await fireEvent.click(screen.getByTitle("Reset zoom"));
    expect(screen.getByTitle("Reset zoom").textContent).toBe("50%");
    picture.kind = "cockpit";
    await tick();
    expect(screen.getByTitle("Reset zoom").textContent).toBe("125%");
    expect(picture.zoomByKind).toEqual({ cockpit: 1_250, overhead: 750, iso25d: 500 });

    // Clicking dead ahead on the first-person canvas locks contact BA (bearing 45°, camera 45°).
    const c = document.querySelector("canvas") as HTMLCanvasElement;
    vi.spyOn(c, "getBoundingClientRect").mockReturnValue({
      left: 0,
      top: 0,
      width: 640,
      height: 480,
      right: 640,
      bottom: 480,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });
    await fireEvent.click(c, { clientX: 320, clientY: 240 });
    expect(helm.mock.calls).toEqual([["LOCK BA"]]);
  });

  it("t walks the fixture's contacts hostiles first, then unidentified, then the rest, wrapping", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    for (let i = 0; i < 4; i++) await press(frame, "t");
    expect(helm.mock.calls).toEqual([["LOCK X3"], ["LOCK X4"], ["LOCK RB"], ["LOCK BA"]]);
  });

  it("T walks back from the frame's lock once the asked lock is stale", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "T", { shiftKey: true });
    expect(helm.mock.calls).toEqual([["LOCK RB"]]);
    // The frame never showed RB: past the window the frame's BA is the lock
    // again.
    vi.advanceTimersByTime(2_000);
    await press(frame, "T", { shiftKey: true });
    expect(helm.mock.calls.at(-1)).toEqual(["LOCK RB"]);
  });

  it("t with no contacts sends nothing", async () => {
    const { helm, frame } = mount(fixtureFrame({ contacts: [] }));
    await tick();
    await press(frame, "t");
    expect(helm).not.toHaveBeenCalled();
  });

  it("F1 prefills ORDER ALL and F2 to F6 the slots, taking the keys from the browser", async () => {
    const { helm, prefill, frame } = mount(fixtureFrame());
    await tick();
    expect(await press(frame, "F1")).toBe(false);
    expect(prefill.mock.calls).toEqual([["ORDER ALL "]]);
    expect(await press(frame, "F5")).toBe(false);
    expect(await press(frame, "F6")).toBe(false);
    expect(prefill.mock.calls.slice(1)).toEqual([["ORDER 5 "], ["ORDER 6 "]]);
    expect(helm).not.toHaveBeenCalled();
  });

  it("uses the projected detachment slot and version for F1 without sending", async () => {
    const cockpit = fixtureFrame({ detachment: {
      id: "7", revision: "8", term: "9", leader_slot: 1,
      slots: [{ slot: 1, callsign: "RA", pilot: "Aster", controller: "player", order_prefill: "ORDER DT:7:8:9 1 " }],
      all_order_prefill: "ORDER DT:7:8:9 ALL ", my_order: null, wilco_command: null, negative_command: null,
    } });
    const { helm, prefill, frame } = mount(cockpit);
    await tick();
    expect(await press(frame, "F1")).toBe(false);
    expect(prefill.mock.calls).toEqual([["ORDER DT:7:8:9 1 "]]);
    expect(await press(frame, "F2")).toBe(false);
    expect(prefill).toHaveBeenCalledTimes(1);
    expect(helm).not.toHaveBeenCalled();
  });

  it("does not fall back to unversioned orders for malformed detachment data", async () => {
    const { helm, prefill, frame } = mount(fixtureFrame({ detachment: { id: "bad" } }));
    await tick();
    expect(await press(frame, "F1")).toBe(false);
    expect(prefill).not.toHaveBeenCalled();
    expect(helm).not.toHaveBeenCalled();
  });

  it("leaves a chord to the browser", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    expect(await press(frame, "w", { ctrlKey: true })).toBe(true);
    expect(await press(frame, "t", { metaKey: true })).toBe(true);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 2);
    expect(helm).not.toHaveBeenCalled();
  });

  it("a and d turn as the arrows do", async () => {
    const { helm, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "a");
    expect(helm.mock.calls).toEqual([["HEADING 44 QUIET"]]);
    vi.advanceTimersByTime(ORDER_GAP_MS * 2);
    await fireEvent.keyUp(frame, { key: "a" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 35 QUIET"]);
    // `d` back from where the picture stands: its tap's degree, then the hold.
    await press(frame, "d");
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 36 QUIET"]);
    vi.advanceTimersByTime(ORDER_GAP_MS * 2);
    await fireEvent.keyUp(frame, { key: "d" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 45 QUIET"]);
  });

  it("q and e swing the look as , and . do", async () => {
    const { picture, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "q");
    vi.advanceTimersByTime(LOOK_STEP_MS);
    expect(picture.lookMdeg).toBe(swingLook(0, -1, LOOK_STEP_MS));
    await press(frame, "q");
    await press(frame, "c");
    expect(picture.lookMdeg).toBe(0);
    await press(frame, "e");
    vi.advanceTimersByTime(LOOK_STEP_MS);
    expect(picture.lookMdeg).toBe(swingLook(0, 1, LOOK_STEP_MS));
  });

  it("v cycles across the three cockpit cameras locally (cockpit → overhead → iso25d → cockpit)", async () => {
    const { picture, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("iso25d");
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("cockpit");
  });

  it("lists the keys in the header", async () => {
    mount(fixtureFrame());
    await tick();
    expect(document.querySelector(".legend")?.textContent?.replace(/\s+/g, " ")).toBe(
      "a d turn · w s speed · x stop · q e look · c centre · v view · f free/hold · g volley · t T lock · n scan · " +
        "b glass · z startup · m emcon · l auto-target · F auto-fire · y N answer · F1–F6 order",
    );
  });
});

describe("PictureView mounted, drawn here: a fresh canvas is painted", () => {
  /** Every canvas's 2D context, recording the frames put on it by the
   * picture drawn here (`paints`). */
  let paints: Map<HTMLCanvasElement, number>;
  let getContext: typeof HTMLCanvasElement.prototype.getContext;

  beforeEach(() => {
    paints = new Map();
    getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
      return {
        putImageData: () => {
          paints.set(this, (paints.get(this) ?? 0) + 1);
        },
      };
    } as unknown as typeof HTMLCanvasElement.prototype.getContext;
    // Animation frames as (fake) timers, stamped with the (fake) clock.
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) =>
      setTimeout(() => cb(Date.now()), 16),
    );
    vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  });

  afterEach(() => {
    HTMLCanvasElement.prototype.getContext = getContext;
  });

  /** The module's promise settled and the effects it wakes run. */
  async function settle(): Promise<void> {
    for (let i = 0; i < 4; i++) await Promise.resolve();
    await tick();
  }

  /** A frame's worth of (fake) time, and the effects a paint wakes. */
  async function frameTime(): Promise<void> {
    vi.advanceTimersByTime(LOCAL_FRAME_MS + 16);
    await tick();
  }

  it("paints the first frame, and the same frame again onto the canvas a hide and show makes", async () => {
    const { unmount } = mount();
    await settle();
    const first = document.querySelector("canvas");
    expect(first).not.toBeNull();
    expect(screen.getByText("Waiting for the first picture…")).toBeTruthy();
    await frameTime();
    expect(paints.get(first as HTMLCanvasElement)).toBe(1);
    expect(screen.queryByText("Waiting for the first picture…")).toBeNull();
    // Unchanged: no further paint.
    await frameTime();
    expect(paints.get(first as HTMLCanvasElement)).toBe(1);

    await fireEvent.click(screen.getByRole("button", { name: "Hide the picture" }));
    expect(document.querySelector("canvas")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Show the picture" }));
    const second = document.querySelector("canvas");
    expect(second).not.toBeNull();
    expect(second).not.toBe(first);
    // Nothing is on the new canvas yet: it says so, and the unchanged frame
    // is painted onto it at the next tick all the same.
    expect(screen.getByText("Waiting for the first picture…")).toBeTruthy();
    await frameTime();
    expect(paints.get(second as HTMLCanvasElement)).toBe(1);
    expect(screen.queryByText("Waiting for the first picture…")).toBeNull();
    // Torn down while the stubs stand: its animation frame is cancelled by
    // the stub that made it.
    unmount();
  });

  it("paints the overhead and 2.5D tactical cameras locally on v and first person again on v back", async () => {
    const setKind = vi.spyOn(fake.module, "setKind");
    const { picture, frame, unmount } = mount(fixtureFrame());
    await settle();
    await frameTime();
    const c = document.querySelector("canvas") as HTMLCanvasElement;
    expect(paints.get(c)).toBe(1);
    expect(setKind).toHaveBeenLastCalledWith(KIND_COCKPIT);

    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    await frameTime();
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    expect(setKind).toHaveBeenLastCalledWith(KIND_OVERHEAD);
    expect(paints.get(c)).toBe(2);

    // Unchanged overhead: no extra paint.
    await frameTime();
    expect(paints.get(c)).toBe(2);

    // Step to 2.5D tactical camera.
    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    await frameTime();
    expect(picture.kind).toBe("iso25d");
    expect(setKind).toHaveBeenLastCalledWith(KIND_ISO25D);
    expect(paints.get(c)).toBe(3);

    // Back to first person: painted here again.
    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    await frameTime();
    expect(picture.kind).toBe("cockpit");
    expect(setKind).toHaveBeenLastCalledWith(KIND_COCKPIT);
    expect(paints.get(c)).toBe(4);
    unmount();
  });

  it("handles cockpit flight keys on document.body without requiring .frame focus, while ignoring text inputs", async () => {
    const { helm, prefill, unmount } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    // Key pressed on document.body (not .frame) still works and calls preventDefault.
    expect(await fireEvent.keyDown(document.body, { key: "w" })).toBe(false);
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"]]);

    // Typing inside an input/textarea is left to the text field (except F1–F6).
    const input = document.createElement("textarea");
    document.body.appendChild(input);
    expect(await fireEvent.keyDown(input, { key: "w" })).toBe(true);
    expect(await fireEvent.keyDown(input, { key: "F2" })).toBe(false);
    expect(prefill.mock.calls).toEqual([["ORDER 2 "]]);
    document.body.removeChild(input);
    unmount();
  });

  it("predicts hull rotation smoothly during a held turn and orders the predicted heading on release", async () => {
    const renderSpy = vi.spyOn(fake.module, "render");
    const { helm, frame, unmount } = mount(fixtureFrame({ heading_deg: 45, power: "online" }));
    await settle();
    await frameTime();
    await fireEvent.keyDown(frame, { key: "d" });
    expect(helm.mock.calls).toEqual([["HEADING 46 QUIET"]]);

    // The swing is a degree in its first 200 ms, then 30°/s: the 16 ms rAF tick at 462 ms
    // rendered +8_860 mdeg of hull offset, with the hold's order of 250 ms (55°) on the tape,
    // and releasing at 500 ms orders the heading the picture shows (45° + 10°).
    vi.advanceTimersByTime(500);
    await tick();
    // The picture opens at 1280x720 unless this browser was told otherwise.
    expect(renderSpy).toHaveBeenLastCalledWith(
      1280,
      720,
      expect.objectContaining({
        offsetMdeg: 8_860,
        hullOffsetMdeg: 8_860,
        orderedMdeg: 55_000,
      }),
    );

    // Releasing the key orders the shown 55° heading, not the stale 45° keyframe heading.
    await fireEvent.keyUp(frame, { key: "d" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 55 QUIET"]);

    // Clicking the HD resolution toggle switches rendering to 640×480, and the
    // choice is kept for the next visit.
    const hdBtn = screen.getByLabelText("HD resolution toggle");
    expect(hdBtn.textContent).toBe("1280p");
    await fireEvent.click(hdBtn);
    expect(hdBtn.textContent).toBe("640p");
    expect(localStorage.getItem("rustmush.pictureHd")).toBe("off");
    await frameTime();
    expect(renderSpy).toHaveBeenLastCalledWith(
      640,
      480,
      expect.any(Object),
    );
    unmount();
    mount(fixtureFrame());
    await tick();
    expect(screen.getByLabelText("HD resolution toggle").textContent).toBe("640p");
  });

  it("switches camera modes via header pills and orders HEADING on HUD compass tape click", async () => {
    const { picture, helm, unmount } = mount(fixtureFrame({ heading_deg: 90, power: "online" }));
    await settle();
    await frameTime();

    await fireEvent.click(screen.getByRole("button", { name: "Overhead" }));
    expect(picture.kind).toBe("overhead");
    await fireEvent.click(screen.getByRole("button", { name: "2.5D" }));
    expect(picture.kind).toBe("iso25d");
    await fireEvent.click(screen.getByRole("button", { name: "1st" }));
    expect(picture.kind).toBe("cockpit");

    const canvas = document.querySelector("canvas") as HTMLCanvasElement;
    canvas.getBoundingClientRect = () =>
      ({
        left: 0,
        top: 0,
        width: 1000,
        height: 500,
      }) as DOMRect;
    // normX = 0.65 (halfway right on the 0.20..0.80 tape => +15° from HEADING=45° = 60°), normY = 0.05
    await fireEvent.click(canvas, { clientX: 650, clientY: 25 });
    expect(helm).toHaveBeenLastCalledWith("HEADING 60 QUIET");
    unmount();
  });

  it("renders the cockpit glass overlay in 1st-person view, toggles it via the Glass button, and shows ballistic cracks when damaged", async () => {
    localStorage.removeItem("rustmush.cockpit_glass");
    const damagedFrame = fixtureFrame();
    const { unmount } = mount(damagedFrame);
    await settle();
    await frameTime();

    const overlay = screen.getByLabelText("cockpit glass overlay");
    expect(overlay).toBeTruthy();
    // Fixture frame has destroyed right_track, so glassDamageLevel is 2.
    expect(overlay.getAttribute("data-damage")).toBe("2");
    expect(overlay.querySelector(".glass-cracks")).not.toBeNull();
    expect(overlay.querySelector(".glass-cracks-heavy")).not.toBeNull();

    const glassBtn = screen.getByRole("button", { name: "cockpit glass" });
    await fireEvent.click(glassBtn);
    expect(screen.queryByLabelText("cockpit glass overlay")).toBeNull();
    expect(localStorage.getItem("rustmush.cockpit_glass")).toBe("off");

    await fireEvent.click(glassBtn);
    expect(screen.getByLabelText("cockpit glass overlay")).toBeTruthy();
    expect(localStorage.getItem("rustmush.cockpit_glass")).toBe("on");
    localStorage.removeItem("rustmush.cockpit_glass");
    unmount();
  });

  it("renders space camera views (Holo-Tank, Space Canopy, 6-DOF Dock) and dispatches 6-DOF docking orders (SF.18)", async () => {
    const setKind = vi.spyOn(fake.module, "setKind");
    const { picture, helm, frame, unmount } = mount(fixtureFrame());
    await settle();
    picture.observeSpaceHolotank({ ship_id: "ship.1", contacts: [] });
    await tick();
    await frameTime();

    expect(picture.spaceMode).toBe("holotank");
    expect(setKind).toHaveBeenLastCalledWith(KIND_HOLOTANK);

    // Cycle space views with `v`: holotank -> space_cockpit -> docking.
    await fireEvent.keyDown(frame, { key: "v" });
    await tick();
    await frameTime();
    expect(picture.spaceMode).toBe("space_cockpit");
    expect(setKind).toHaveBeenLastCalledWith(KIND_SPACE_COCKPIT);

    await fireEvent.keyDown(frame, { key: "v" });
    await tick();
    await frameTime();
    expect(picture.spaceMode).toBe("docking");
    expect(setKind).toHaveBeenLastCalledWith(KIND_DOCKING);

    // Non-colliding 6-DOF docking keyboard orders and toolbar buttons (`D-35-16`).
    await fireEvent.keyDown(frame, { key: "w" });
    await fireEvent.keyDown(frame, { key: "ArrowUp" });
    await fireEvent.keyDown(frame, { key: "q" });
    await fireEvent.keyDown(frame, { key: "r" });
    await fireEvent.keyDown(frame, { key: "x" });
    expect(helm.mock.calls).toEqual([
      ["DOCK TRANS UP"],
      ["DOCK PITCH UP"],
      ["DOCK ROLL CCW"],
      ["DOCK CLOSURE FORWARD"],
      ["DOCK DAMPENERS TOGGLE"],
    ]);

    const dampBtn = screen.getByRole("button", { name: "Dampeners (X)" });
    await fireEvent.click(dampBtn);
    expect(helm).toHaveBeenLastCalledWith("DOCK DAMPENERS TOGGLE");

    await fireEvent.click(screen.getByRole("button", { name: "Holo-Tank" }));
    expect(picture.spaceMode).toBe("holotank");
    unmount();
  });
});
