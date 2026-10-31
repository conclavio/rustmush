// @vitest-environment jsdom
//
// PictureView mounted (doc 21a `X-02`): what the arrow keys hold is let go
// when their release can no longer be seen. The app is the picture's store
// and the two spies the view may reach for (`setPicture`, `helm`); the
// picture is drawn by the server here (jsdom cannot draw it), asked for the
// moment the view mounts wanting it. Fake timers drive the helm's repeats
// and the look's steps.
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
import { decodeCockpitFrame, type CockpitFrame } from "$lib/cockpit";
import type { GameApp } from "$lib/game.svelte";
import {
  HELM_REPEAT_MS,
  LOOK_STEP_MS,
  PICTURE_PREF_KEY,
  SPEED_COALESCE_MS,
  swingLook,
  type PictureKind,
} from "$lib/picture";
import { LOCAL_FRAME_MS } from "$lib/pixel/local";
import { KIND_COCKPIT, KIND_ISO25D, KIND_OVERHEAD } from "$lib/pixel/wasm";
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
      hash: () => (kind === 2 ? "overhead frame" : kind === 6 ? "iso25d frame" : "cockpit frame"),
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

/** The fixture tank's heading, so a turn is `HEADING 15` or `75`, and a stop
 * `HEADING 45`. */
const HEADING = 45;

function mount(cockpit: CockpitFrame | null = null): {
  picture: PictureStore;
  setPicture: ReturnType<typeof vi.fn>;
  helm: ReturnType<typeof vi.fn>;
  prefill: ReturnType<typeof vi.fn>;
  frame: HTMLElement;
  unmount: () => void;
} {
  const picture = new PictureStore();
  const setPicture = vi.fn((on: boolean, look: number, kind?: PictureKind) => {
    picture.on = on;
    picture.lookMdeg = look;
    if (kind !== undefined) picture.kind = kind;
  });
  const helm = vi.fn();
  const prefill = vi.fn();
  const app = { picture, setPicture, helm } as unknown as GameApp;
  const { unmount } = render(PictureView, {
    props: { app, headingDeg: HEADING, frame: cockpit, prefill },
  });
  return { picture, setPicture, helm, prefill, frame: screen.getByRole("application"), unmount };
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
  delete (document as { visibilityState?: unknown }).visibilityState;
});

describe("PictureView mounted: a held key is let go when its release cannot be seen", () => {
  it("orders the turn while the arrow is down, and repeats it", async () => {
    const { picture, helm, frame } = mount();
    await tick();
    expect(picture.on).toBe(true);
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"]]);
    vi.advanceTimersByTime(HELM_REPEAT_MS);
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"], ["HEADING 15 QUIET"]]);
    await fireEvent.keyUp(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 45 QUIET"]);
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    expect(helm.mock.calls).toHaveLength(3);
  });

  it("stops the turn when the page goes hidden, and orders nothing more", async () => {
    const { picture, helm, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowRight" });
    expect(helm.mock.calls).toEqual([["HEADING 75 QUIET"]]);
    setVisibility("hidden");
    await tick();
    expect(helm.mock.calls).toEqual([["HEADING 75 QUIET"], ["HEADING 45 QUIET"]]);
    // The release of the arrow, out of sight, changes nothing; nor do the
    // repeats that would have run.
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    await fireEvent.keyUp(frame, { key: "ArrowRight" });
    expect(helm.mock.calls).toHaveLength(2);
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
    window.dispatchEvent(new Event("blur"));
    await tick();
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"], ["HEADING 45 QUIET"]]);
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    expect(helm.mock.calls).toHaveLength(2);
  });

  it("lets go when torn down mid-turn, as a blur would", async () => {
    const { helm, frame, unmount } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"]]);
    unmount();
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"], ["HEADING 45 QUIET"]]);
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    expect(helm.mock.calls).toHaveLength(2);
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
    expect(picture.lookMdeg).toBe(0);
    vi.advanceTimersByTime(LOOK_STEP_MS * 5);
    expect(picture.lookMdeg).toBe(0);
  });
});

describe("PictureView mounted: the cockpit's keys (doc 21 §7.3, cut G.12c)", () => {
  /** A key pressed on the picture; `false` when the view took it from the
   * browser (`preventDefault`). */
  async function press(el: HTMLElement, key: string, init: KeyboardEventInit = {}): Promise<boolean> {
    return fireEvent.keyDown(el, { key, ...init });
  }

  it("w from rest orders a tenth of the top speed once the window closes, and shows it", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    expect(await press(frame, "w")).toBe(false);
    // Coalesced: nothing yet.
    expect(helm).not.toHaveBeenCalled();
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
    expect(helm.mock.calls).toEqual([["SPEED 6 QUIET"]]);
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

  it("two w presses inside the window send one line, the second value", async () => {
    const { helm, frame } = mount(fixtureFrame({ max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "w");
    vi.advanceTimersByTime(SPEED_COALESCE_MS / 2);
    await press(frame, "w", { repeat: true });
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
    expect(helm.mock.calls).toEqual([["SPEED 12 QUIET"]]);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 4);
    expect(helm).toHaveBeenCalledTimes(1);
  });

  it("x orders SPEED 0 at once, drops a waiting step, and stops the helm", async () => {
    const { helm, frame } = mount(fixtureFrame({ speed_kph: 30, max_speed_kph: 60, reverse_kph: 20 }));
    await tick();
    await press(frame, "w");
    await press(frame, "x");
    expect(helm.mock.calls).toEqual([["SPEED 0 QUIET"], ["HEADING 45 QUIET"]]);
    vi.advanceTimersByTime(SPEED_COALESCE_MS * 2);
    expect(helm).toHaveBeenCalledTimes(2);
    await tick();
    expect(screen.getByLabelText("speed ordered").textContent).toBe("spd 30→0");
  });

  it("f under hold orders FIRE FREE, and the header says hold", async () => {
    const { helm, frame } = mount(fixtureFrame({ weapons_free: false }));
    await tick();
    expect(screen.getByText("weapons hold")).toBeTruthy();
    await press(frame, "f");
    expect(helm.mock.calls).toEqual([["FIRE FREE"]]);
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
    expect(helm.mock.calls).toEqual([["HEADING 15 QUIET"]]);
    await fireEvent.keyUp(frame, { key: "a" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 45 QUIET"]);
    await press(frame, "d");
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 75 QUIET"]);
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
    const { picture, setPicture, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    expect(setPicture).not.toHaveBeenCalled();
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
      "a d turn · w s speed · q e look · c centre · x stop · v view · f free/hold · g volley · t lock · F1–F6 order",
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
    const { setPicture, unmount } = mount();
    await settle();
    // Drawn here: the server is never asked.
    expect(setPicture).not.toHaveBeenCalled();
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
    const { picture, setPicture, frame, unmount } = mount(fixtureFrame());
    await settle();
    await frameTime();
    const c = document.querySelector("canvas") as HTMLCanvasElement;
    expect(paints.get(c)).toBe(1);
    expect(setKind).toHaveBeenLastCalledWith(KIND_COCKPIT);
    expect(setPicture).not.toHaveBeenCalled();

    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    await frameTime();
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    expect(setKind).toHaveBeenLastCalledWith(KIND_OVERHEAD);
    expect(paints.get(c)).toBe(2);
    expect(setPicture).not.toHaveBeenCalled();

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

    // Back to first person: painted here again, the server still never asked.
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
    vi.advanceTimersByTime(SPEED_COALESCE_MS);
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
    expect(helm.mock.calls).toEqual([["HEADING 75 QUIET"]]);

    // After 500 ms at 30°/s, the 16 ms rAF tick at 462 ms rendered +13_860 mdeg hull offset,
    // and releasing at 500 ms orders the predicted 60° heading (45° + 15°).
    vi.advanceTimersByTime(500);
    await tick();
    expect(renderSpy).toHaveBeenLastCalledWith(
      640,
      480,
      expect.objectContaining({
        offsetMdeg: 13_860,
        hullOffsetMdeg: 13_860,
        orderedMdeg: 75_000,
      }),
    );

    // Releasing the key orders the predicted 60° heading, not the stale 45° keyframe heading.
    await fireEvent.keyUp(frame, { key: "d" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 60 QUIET"]);
    unmount();
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
});
