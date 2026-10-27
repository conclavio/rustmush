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
import { LOCAL_FRAME_MS, PICTURE_SOURCE_KEY } from "$lib/pixel/local";
import { PictureStore } from "$lib/stores/picture.svelte";

/** The engine's module, faked: every render succeeds and hashes the same,
 * as an unchanged scene does. */
const fake = vi.hoisted(() => ({
  module: {
    feed: () => true,
    render: () => true,
    hash: () => "the same frame",
    image: () => ({ width: 1, height: 1, data: new Uint8ClampedArray(4) }),
    pixels: () => new Uint8Array(0),
    reset: () => {},
  },
}));

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
  // The real `setPicture`'s bookkeeping, less the socket: a camera given is
  // kept, none leaves the store's, as `GameApp.setPicture` does (doc 21 cut
  // G.12c).
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
  localStorage.setItem(PICTURE_PREF_KEY, "on");
  localStorage.setItem(PICTURE_SOURCE_KEY, "server");
});

afterEach(() => {
  // The stubs first (they were put over the fake timers' own globals), the
  // real timers after, so nothing of the fake clock is left behind.
  vi.unstubAllGlobals();
  vi.useRealTimers();
  localStorage.removeItem(PICTURE_PREF_KEY);
  localStorage.removeItem(PICTURE_SOURCE_KEY);
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
    const { picture, setPicture, helm, frame } = mount();
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
    // The picture itself is stopped while hidden and asked for again after.
    expect(picture.on).toBe(false);
    setVisibility("visible");
    await tick();
    expect(setPicture.mock.calls.at(-1)).toEqual([true, 0]);
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
    const { setPicture, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "," });
    vi.advanceTimersByTime(LOOK_STEP_MS);
    const step = swingLook(0, -1, LOOK_STEP_MS);
    expect(setPicture.mock.calls.at(-1)).toEqual([true, step]);
    setVisibility("hidden");
    await tick();
    expect(setPicture.mock.calls.at(-1)).toEqual([false, 0]);
    const asked = setPicture.mock.calls.length;
    vi.advanceTimersByTime(LOOK_STEP_MS * 5);
    expect(setPicture.mock.calls).toHaveLength(asked);
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
    const { setPicture, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "q");
    vi.advanceTimersByTime(LOOK_STEP_MS);
    expect(setPicture.mock.calls.at(-1)).toEqual([true, swingLook(0, -1, LOOK_STEP_MS)]);
    await press(frame, "q");
    await press(frame, "c");
    expect(setPicture.mock.calls.at(-1)).toEqual([true, 0]);
    await press(frame, "e");
    vi.advanceTimersByTime(LOOK_STEP_MS);
    expect(setPicture.mock.calls.at(-1)).toEqual([true, swingLook(0, 1, LOOK_STEP_MS)]);
  });

  it("v asks the gateway for the overhead and back, the server's frame shown meanwhile", async () => {
    const { picture, setPicture, frame } = mount(fixtureFrame());
    await tick();
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    expect(setPicture.mock.calls.at(-1)?.[0]).toBe(true);
    expect(screen.getByRole("button", { name: "overhead: drawn by the server" })).toBeTruthy();
    await press(frame, "v");
    await tick();
    expect(picture.kind).toBe("cockpit");
    expect(screen.getByRole("button", { name: "drawn by the server" })).toBeTruthy();
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
   * picture drawn here (`paints`) and by the server's bitmaps (`draws`). */
  let paints: Map<HTMLCanvasElement, number>;
  let draws: Map<HTMLCanvasElement, number>;
  let getContext: typeof HTMLCanvasElement.prototype.getContext;

  beforeEach(() => {
    localStorage.setItem(PICTURE_SOURCE_KEY, "here");
    paints = new Map();
    draws = new Map();
    getContext = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement) {
      return {
        putImageData: () => {
          paints.set(this, (paints.get(this) ?? 0) + 1);
        },
        drawImage: () => {
          draws.set(this, (draws.get(this) ?? 0) + 1);
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
    expect(setPicture.mock.calls.filter((c) => c[0] === true)).toHaveLength(0);
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

  it("stands aside for the overhead, which only the gateway draws (doc 21 cut G.12c)", async () => {
    const { picture, setPicture, frame, unmount } = mount(fixtureFrame());
    await settle();
    await frameTime();
    const c = document.querySelector("canvas") as HTMLCanvasElement;
    expect(paints.get(c)).toBe(1);
    expect(setPicture.mock.calls.filter((call) => call[0] === true)).toHaveLength(0);
    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    // The gateway is asked, for the overhead; nothing more is painted here.
    expect(picture.kind).toBe("overhead");
    expect(picture.on).toBe(true);
    expect(screen.getByRole("button", { name: "overhead: drawn by the server" })).toBeTruthy();
    picture.receive(new Blob([new Uint8Array([0x89, 0x50])], { type: "image/png" }));
    await frameTime();
    await frameTime();
    expect(paints.get(c)).toBe(1);
    // Back to first person: drawn here again, the gateway's picture stopped.
    await fireEvent.keyDown(frame, { key: "v" });
    await settle();
    expect(picture.kind).toBe("cockpit");
    expect(picture.on).toBe(false);
    expect(screen.getByRole("button", { name: "drawn here" })).toBeTruthy();
    unmount();
  });

  it("paints the unchanged frame again over a server frame once drawn here again", async () => {
    // The server's frames are drawn through createImageBitmap, which jsdom
    // lacks: a bitmap of the frame's size.
    vi.stubGlobal("createImageBitmap", () =>
      Promise.resolve({ width: 640, height: 480, close: () => {} }),
    );
    const { picture, unmount } = mount();
    await settle();
    await frameTime();
    const c = document.querySelector("canvas") as HTMLCanvasElement;
    expect(paints.get(c)).toBe(1);
    // The viewer chooses the server's picture; its frame lands on the same
    // canvas.
    await fireEvent.click(screen.getByRole("button", { name: "drawn here" }));
    expect(picture.on).toBe(true);
    picture.receive(new Blob([new Uint8Array([0x89, 0x50])], { type: "image/png" }));
    await settle();
    expect(draws.get(c)).toBe(1);
    // Back to drawn here: the picture's frame is unchanged, and goes over
    // the server's all the same.
    await fireEvent.click(screen.getByRole("button", { name: "drawn by the server" }));
    await settle();
    await frameTime();
    expect(paints.get(c)).toBe(2);
    unmount();
  });
});
