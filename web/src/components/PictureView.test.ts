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

import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import PictureView from "./PictureView.svelte";
import type { GameApp } from "$lib/game.svelte";
import { HELM_REPEAT_MS, LOOK_STEP_MS, PICTURE_PREF_KEY, swingLook } from "$lib/picture";
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

function mount(): {
  picture: PictureStore;
  setPicture: ReturnType<typeof vi.fn>;
  helm: ReturnType<typeof vi.fn>;
  frame: HTMLElement;
  unmount: () => void;
} {
  const picture = new PictureStore();
  const setPicture = vi.fn((on: boolean, look: number) => {
    picture.on = on;
    picture.lookMdeg = look;
  });
  const helm = vi.fn();
  const app = { picture, setPicture, helm } as unknown as GameApp;
  const { unmount } = render(PictureView, { props: { app, headingDeg: HEADING } });
  return { picture, setPicture, helm, frame: screen.getByRole("application"), unmount };
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
    expect(helm.mock.calls).toEqual([["HEADING 15"]]);
    vi.advanceTimersByTime(HELM_REPEAT_MS);
    expect(helm.mock.calls).toEqual([["HEADING 15"], ["HEADING 15"]]);
    await fireEvent.keyUp(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls.at(-1)).toEqual(["HEADING 45"]);
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    expect(helm.mock.calls).toHaveLength(3);
  });

  it("stops the turn when the page goes hidden, and orders nothing more", async () => {
    const { picture, setPicture, helm, frame } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowRight" });
    expect(helm.mock.calls).toEqual([["HEADING 75"]]);
    setVisibility("hidden");
    await tick();
    expect(helm.mock.calls).toEqual([["HEADING 75"], ["HEADING 45"]]);
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
    expect(helm.mock.calls).toEqual([["HEADING 15"], ["HEADING 45"]]);
    vi.advanceTimersByTime(HELM_REPEAT_MS * 3);
    expect(helm.mock.calls).toHaveLength(2);
  });

  it("lets go when torn down mid-turn, as a blur would", async () => {
    const { helm, frame, unmount } = mount();
    await tick();
    await fireEvent.keyDown(frame, { key: "ArrowLeft" });
    expect(helm.mock.calls).toEqual([["HEADING 15"]]);
    unmount();
    expect(helm.mock.calls).toEqual([["HEADING 15"], ["HEADING 45"]]);
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
