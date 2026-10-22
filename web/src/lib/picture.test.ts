// The cockpit's picture in the web client (doc 21 cut G.3c): the command, the
// look's swing, the helm's lines and the keys — pure, so pinned here.

import { describe, expect, it } from "vitest";

import {
  helmLine,
  LOOK_MDEG_PER_S,
  pictureCommand,
  pictureKey,
  swingLook,
  wrapMdeg,
} from "./picture";
import { PictureStore } from "./stores/picture.svelte";

describe("the picture command", () => {
  it("carries on/off and a wrapped look", () => {
    expect(pictureCommand(true, 30_000)).toEqual({ t: "picture", d: { on: true, look_mdeg: 30_000 } });
    expect(pictureCommand(false, 0)).toEqual({ t: "picture", d: { on: false, look_mdeg: 0 } });
    expect(pictureCommand(true, 200_000).d).toEqual({ on: true, look_mdeg: -160_000 });
  });

  it("wraps a look into -180..180 degrees", () => {
    expect(wrapMdeg(0)).toBe(0);
    expect(wrapMdeg(180_000)).toBe(-180_000);
    expect(wrapMdeg(-190_000)).toBe(170_000);
    expect(wrapMdeg(720_500)).toBe(500);
  });
});

describe("the look and the helm", () => {
  it("swings the look at 45 degrees a second, a second at most a step", () => {
    expect(LOOK_MDEG_PER_S).toBe(45_000);
    expect(swingLook(0, 1, 100)).toBe(4_500);
    expect(swingLook(0, -1, 100)).toBe(-4_500);
    expect(swingLook(178_000, 1, 100)).toBe(-177_500);
    expect(swingLook(0, 1, 5_000)).toBe(45_000);
    expect(swingLook(1_000, 0, 100)).toBe(1_000);
  });

  it("orders a heading thirty degrees ahead, or where the tank points to stop", () => {
    expect(helmLine(90, 1)).toBe("HEADING 120");
    expect(helmLine(10, -1)).toBe("HEADING 340");
    expect(helmLine(350, 1)).toBe("HEADING 20");
    expect(helmLine(45, 0)).toBe("HEADING 45");
  });

  it("reads the keys the TUI's view reads", () => {
    expect(pictureKey("ArrowLeft")).toEqual({ kind: "turn", dir: -1 });
    expect(pictureKey("ArrowRight")).toEqual({ kind: "turn", dir: 1 });
    expect(pictureKey(",")).toEqual({ kind: "look", dir: -1 });
    expect(pictureKey(".")).toEqual({ kind: "look", dir: 1 });
    expect(pictureKey("c")).toEqual({ kind: "centre" });
    expect(pictureKey(" ")).toEqual({ kind: "stop" });
    expect(pictureKey("a")).toBeNull();
  });
});

describe("the picture store", () => {
  it("keeps frames only while the picture is on", () => {
    const store = new PictureStore();
    const png = new Uint8Array([0x89, 0x50]).buffer;
    store.receive(png);
    expect(store.frame).toBeNull();
    store.on = true;
    store.receive(png);
    expect(store.frame).toBeInstanceOf(Blob);
    expect(store.frame?.type).toBe("image/png");
    expect(store.received).toBe(1);
    store.clear();
    expect(store.frame).toBeNull();
    expect(store.received).toBe(0);
  });
});
