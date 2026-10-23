// @vitest-environment jsdom
//
// PictureView mounted (doc 21a `X-02`): what the arrow keys hold is let go
// when their release can no longer be seen. The app is the picture's store
// and the two spies the view may reach for (`setPicture`, `helm`); the
// picture is drawn by the server here (jsdom cannot draw it), asked for the
// moment the view mounts wanting it. Fake timers drive the helm's repeats
// and the look's steps.

import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import PictureView from "./PictureView.svelte";
import type { GameApp } from "$lib/game.svelte";
import { HELM_REPEAT_MS, LOOK_STEP_MS, PICTURE_PREF_KEY, swingLook } from "$lib/picture";
import { PICTURE_SOURCE_KEY } from "$lib/pixel/local";
import { PictureStore } from "$lib/stores/picture.svelte";

/** The fixture tank's heading, so a turn is `HEADING 15` or `75`, and a stop
 * `HEADING 45`. */
const HEADING = 45;

function mount(): {
  picture: PictureStore;
  setPicture: ReturnType<typeof vi.fn>;
  helm: ReturnType<typeof vi.fn>;
  frame: HTMLElement;
} {
  const picture = new PictureStore();
  const setPicture = vi.fn((on: boolean, look: number) => {
    picture.on = on;
    picture.lookMdeg = look;
  });
  const helm = vi.fn();
  const app = { picture, setPicture, helm } as unknown as GameApp;
  render(PictureView, { props: { app, headingDeg: HEADING } });
  return { picture, setPicture, helm, frame: screen.getByRole("application") };
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
