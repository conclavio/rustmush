// The cockpit's picture in the web client (doc 21 cut G.3c): the command, the
// look's swing, the helm's lines and the keys — pure, so pinned here.

//
// Doc 21 cut G.12c: every key of §7.3's table the web binds, one assertion a
// key, and the rules behind `w`/`s` (the step, the clamp), `f` (the gate) and
// `t`/`T` (the lock order) against the committed fixture's contacts.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

import { decodeCockpitFrame, type CockpitContact } from "./cockpit";
import {
  canvasClickHeadingDeg,
  gateLine,
  gateWord,
  helmLine,
  lockOrder,
  LOOK_MDEG_PER_S,
  nextLock,
  nextPictureKind,
  nextSpaceCamera,
  orderPrefill,
  pickCanvasContact,
  pictureCommand,
  pictureKey,
  predictedOffsetCm,
  spacePictureKey,
  speedLine,
  speedOutstanding,
  speedStep,
  steppedSpeed,
  stepZoomPermille,
  swingLook,
  wrapMdeg,
} from "./picture";
import { PictureStore } from "./stores/picture.svelte";

/** The committed fixture frame's contacts: hostile BA at 450 m, friendly RB
 * at 100 m, unidentified X3 at 200 m and X4 at 600 m; BA is locked. */
function fixtureContacts(): CockpitContact[] {
  const raw: unknown = JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/cockpit_frame.json"), "utf8"),
  );
  return decodeCockpitFrame(raw)!.contacts;
}

describe("the picture command", () => {
  it("carries on/off, a wrapped look and the camera", () => {
    expect(pictureCommand(true, 30_000)).toEqual({
      t: "picture",
      d: { on: true, look_mdeg: 30_000, kind: "cockpit" },
    });
    expect(pictureCommand(false, 0)).toEqual({
      t: "picture",
      d: { on: false, look_mdeg: 0, kind: "cockpit" },
    });
    expect(pictureCommand(true, 200_000).d).toEqual({ on: true, look_mdeg: -160_000, kind: "cockpit" });
    // Doc 21 cut G.12c: the overhead is asked for by name.
    expect(pictureCommand(true, 0, "overhead").d).toEqual({ on: true, look_mdeg: 0, kind: "overhead" });
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
    expect(helmLine(90, 1)).toBe("HEADING 120 QUIET");
    expect(helmLine(10, -1)).toBe("HEADING 340 QUIET");
    expect(helmLine(350, 1)).toBe("HEADING 20 QUIET");
    expect(helmLine(45, 0)).toBe("HEADING 45 QUIET");
  });

  it("reads the keys the TUI's view reads", () => {
    expect(pictureKey("ArrowLeft")).toEqual({ kind: "turn", dir: -1 });
    expect(pictureKey("ArrowRight")).toEqual({ kind: "turn", dir: 1 });
    expect(pictureKey(",")).toEqual({ kind: "look", dir: -1 });
    expect(pictureKey(".")).toEqual({ kind: "look", dir: 1 });
    expect(pictureKey("c")).toEqual({ kind: "centre" });
    expect(pictureKey(" ")).toEqual({ kind: "steady" });
    expect(pictureKey("z")).toBeNull();
  });
});

describe("the cockpit's keys (doc 21 §7.3, cut G.12c)", () => {
  it("a and d turn, as the arrows do", () => {
    expect(pictureKey("a")).toEqual({ kind: "turn", dir: -1 });
    expect(pictureKey("d")).toEqual({ kind: "turn", dir: 1 });
  });

  it("w and s step the speed", () => {
    expect(pictureKey("w")).toEqual({ kind: "speed", dir: 1 });
    expect(pictureKey("s")).toEqual({ kind: "speed", dir: -1 });
  });

  it("x is all stop", () => {
    expect(pictureKey("x")).toEqual({ kind: "allStop" });
  });

  it("q and e swing the look, beside the kept , and .", () => {
    expect(pictureKey("q")).toEqual({ kind: "look", dir: -1 });
    expect(pictureKey("e")).toEqual({ kind: "look", dir: 1 });
  });

  it("c centres the look", () => {
    expect(pictureKey("c")).toEqual({ kind: "centre" });
  });

  it("space is steady", () => {
    expect(pictureKey(" ")).toEqual({ kind: "steady" });
  });

  it("v switches the camera", () => {
    expect(pictureKey("v")).toEqual({ kind: "view" });
    expect(nextPictureKind("cockpit")).toBe("overhead");
    expect(nextPictureKind("overhead")).toBe("iso25d");
    expect(nextPictureKind("iso25d")).toBe("cockpit");
  });

  it("f flips the weapons gate", () => {
    expect(pictureKey("f")).toEqual({ kind: "gate" });
  });

  it("g is the volley", () => {
    expect(pictureKey("g")).toEqual({ kind: "volley" });
  });

  it("t locks the next contact and T the previous", () => {
    expect(pictureKey("t")).toEqual({ kind: "lock", dir: 1 });
    expect(pictureKey("T")).toEqual({ kind: "lock", dir: -1 });
  });

  it("F1 opens ORDER ALL and F2 to F6 the slots", () => {
    expect(pictureKey("F1")).toEqual({ kind: "order", slot: "ALL" });
    expect(pictureKey("F2")).toEqual({ kind: "order", slot: 2 });
    expect(pictureKey("F3")).toEqual({ kind: "order", slot: 3 });
    expect(pictureKey("F4")).toEqual({ kind: "order", slot: 4 });
    expect(pictureKey("F5")).toEqual({ kind: "order", slot: 5 });
    expect(pictureKey("F6")).toEqual({ kind: "order", slot: 6 });
    expect(pictureKey("F7")).toBeNull();
    expect(orderPrefill("ALL")).toBe("ORDER ALL ");
    expect(orderPrefill(4)).toBe("ORDER 4 ");
  });

  it("reads a letter in either case but t, whose capital is the previous lock", () => {
    for (const k of ["a", "d", "w", "s", "x", "q", "e", "c", "v", "f", "g"]) {
      expect(pictureKey(k.toUpperCase()), k).toEqual(pictureKey(k));
    }
    expect(pictureKey("T")).not.toEqual(pictureKey("t"));
  });
});

describe("the speed's step and clamp (doc 21 cut G.12c)", () => {
  it("steps a tenth of the top speed, at least 5 kph", () => {
    expect(speedStep(60)).toBe(6);
    expect(speedStep(120)).toBe(12);
    expect(speedStep(30)).toBe(5);
    expect(speedStep(85)).toBe(9);
  });

  it("steps through zero into reverse and clamps at either ceiling", () => {
    expect(steppedSpeed(0, 1, 60, 20)).toBe(6);
    expect(steppedSpeed(58, 1, 60, 20)).toBe(60);
    expect(steppedSpeed(60, 1, 60, 20)).toBe(60);
    expect(steppedSpeed(3, -1, 60, 20)).toBe(-3);
    expect(steppedSpeed(0, -1, 60, 20)).toBe(-6);
    expect(steppedSpeed(-18, -1, 60, 20)).toBe(-20);
    expect(steppedSpeed(-20, -1, 60, 20)).toBe(-20);
  });

  it("holds an order outstanding until the frame is within a kph of it", () => {
    expect(speedOutstanding(null, 0)).toBe(false);
    expect(speedOutstanding(6, 0)).toBe(true);
    expect(speedOutstanding(6, 5)).toBe(false);
    expect(speedOutstanding(6, 7)).toBe(false);
    expect(speedOutstanding(-6, 0)).toBe(true);
    expect(speedLine(-6)).toBe("SPEED -6 QUIET");
  });
});

describe("the weapons gate (doc 21 cut G.12c)", () => {
  it("orders free from hold and hold from free, a missing key reading free", () => {
    expect(gateLine(false)).toBe("FIRE FREE");
    expect(gateLine(true)).toBe("FIRE HOLD");
    expect(gateLine(null)).toBe("FIRE HOLD");
    expect(gateWord(false)).toBe("weapons hold");
    expect(gateWord(true)).toBe("weapons free");
    expect(gateWord(null)).toBe("weapons free");
  });
});

describe("the lock cycle (doc 21 cut G.12c)", () => {
  it("orders hostiles first, then unidentified, then the rest, nearest first in each", () => {
    expect(lockOrder(fixtureContacts())).toEqual(["BA", "X3", "X4", "RB"]);
  });

  it("walks forward with t and back with T, wrapping, from the current lock", () => {
    const contacts = fixtureContacts();
    expect(nextLock(contacts, "BA", 1)).toBe("X3");
    expect(nextLock(contacts, "X3", 1)).toBe("X4");
    expect(nextLock(contacts, "X4", 1)).toBe("RB");
    expect(nextLock(contacts, "RB", 1)).toBe("BA");
    expect(nextLock(contacts, "BA", -1)).toBe("RB");
    expect(nextLock(contacts, "X3", -1)).toBe("BA");
  });

  it("starts at the first from no lock or a lock no longer listed, and is none with no contacts", () => {
    const contacts = fixtureContacts();
    expect(nextLock(contacts, null, 1)).toBe("BA");
    expect(nextLock(contacts, null, -1)).toBe("BA");
    expect(nextLock(contacts, "ZZ", 1)).toBe("BA");
    expect(nextLock([], null, 1)).toBeNull();
    expect(nextLock([], "BA", -1)).toBeNull();
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

describe("the keyframes the picture is drawn from here (doc 21 cut G.3b)", () => {
  it("keeps the latest cockpit and map, and drops the cockpit when the seat goes", () => {
    const store = new PictureStore();
    const cockpit = { match: 7, seated: true };
    const map = { match: 7, width: 96 };
    store.observeCockpit(cockpit);
    store.observeMap(map);
    expect(store.cockpitData).toBe(cockpit);
    expect(store.mapData).toBe(map);
    store.observeCockpit({ seated: false });
    expect(store.cockpitData).toBeNull();
    expect(store.mapData).toBe(map);
  });

  it("dead-reckons 20 Hz position offsets along the predicted heading and clamps at 1.5 pulses", () => {
    expect(predictedOffsetCm(0, 90_000, 500)).toEqual({ dxCm: 0, dyCm: 0 });
    expect(predictedOffsetCm(1_000, 0, 500)).toEqual({ dxCm: 0, dyCm: 500 });
    expect(predictedOffsetCm(1_000, 90_000, 500)).toEqual({ dxCm: 500, dyCm: 0 });
    expect(predictedOffsetCm(1_000, 90_000, 5_000, 1_500)).toEqual({ dxCm: 1_500, dyCm: 0 });
  });

  it("steps 2.5D zoom between 500 and 2000 permille and picks contacts or headings on tactical canvas", () => {
    expect(pictureKey("+")).toEqual({ kind: "zoom", dir: 1 });
    expect(pictureKey("-")).toEqual({ kind: "zoom", dir: -1 });
    expect(stepZoomPermille(1_000, 1)).toBe(1_250);
    expect(stepZoomPermille(2_000, 1)).toBe(2_000);
    expect(stepZoomPermille(500, -1)).toBe(500);
    expect(stepZoomPermille(1_500, 0)).toBe(1_000);

    const contacts = fixtureContacts();
    // Friendly RB is at range 100m, bearing 270 deg (due west) -> in iso25d (1000 permille):
    // dx_m = -100m -> sx = 320 - 100 * 2.3 = 90 -> normX = 90 / 640, sy = 240 -> normY = 0.5
    expect(pickCanvasContact(contacts, "iso25d", 90 / 640, 240 / 480, 1_000)).toBe("RB");
    expect(pickCanvasContact(contacts, "cockpit", 90 / 640, 240 / 480, 1_000)).toBeNull();

    // Double-click heading: straight north of centre (0.5, 0.2) -> 0 deg; straight east (0.8, 0.5) -> 90 deg
    expect(canvasClickHeadingDeg("iso25d", 0.5, 0.2)).toBe(0);
    expect(canvasClickHeadingDeg("iso25d", 0.8, 0.5)).toBe(90);
    expect(canvasClickHeadingDeg("iso25d", 0.5, 0.5)).toBeNull();
    expect(canvasClickHeadingDeg("cockpit", 0.8, 0.5)).toBeNull();
  });

  it("cycles space cameras and maps non-colliding 6-DOF docking and space bridge keys (SF.18, D-35-16)", () => {
    expect(nextSpaceCamera("holotank")).toBe("space_cockpit");
    expect(nextSpaceCamera("space_cockpit")).toBe("docking");
    expect(nextSpaceCamera("docking")).toBe("holotank");

    // 6-DOF docking keys match `crates/mush-net-ssh/src/tui/pixel.rs` and leave digits 0..9 free.
    expect(spacePictureKey("w", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK TRANS UP" });
    expect(spacePictureKey("s", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK TRANS DOWN" });
    expect(spacePictureKey("a", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK TRANS LEFT" });
    expect(spacePictureKey("d", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK TRANS RIGHT" });
    expect(spacePictureKey("ArrowUp", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK PITCH UP",
    });
    expect(spacePictureKey("ArrowDown", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK PITCH DOWN",
    });
    expect(spacePictureKey("ArrowLeft", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK YAW LEFT",
    });
    expect(spacePictureKey("ArrowRight", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK YAW RIGHT",
    });
    expect(spacePictureKey("q", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK ROLL CCW" });
    expect(spacePictureKey("e", "docking")).toEqual({ kind: "dockOrder", cmd: "DOCK ROLL CW" });
    expect(spacePictureKey("r", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK CLOSURE FORWARD",
    });
    expect(spacePictureKey("f", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK CLOSURE REVERSE",
    });
    expect(spacePictureKey("x", "docking")).toEqual({
      kind: "dockOrder",
      cmd: "DOCK DAMPENERS TOGGLE",
    });
    expect(spacePictureKey("v", "docking")).toEqual({ kind: "spaceView" });
    expect(spacePictureKey("0", "docking")).toBeNull();
    expect(spacePictureKey("9", "docking")).toBeNull();

    // Holo-Tank orbit & Space Canopy helm keys.
    expect(spacePictureKey("a", "holotank")).toEqual({
      kind: "holotankOrbit",
      deltaMdeg: -15_000,
    });
    expect(spacePictureKey("d", "holotank")).toEqual({
      kind: "holotankOrbit",
      deltaMdeg: 15_000,
    });
    expect(spacePictureKey("w", "space_cockpit")).toEqual({
      kind: "spaceHelm",
      cmd: "+helm/burn 10 0 0",
    });
    expect(spacePictureKey("g", "space_cockpit")).toEqual({
      kind: "spaceHelm",
      cmd: "+tac/decoy",
    });

    // PictureStore space feed observers auto-select spaceMode.
    const store = new PictureStore();
    expect(store.hasSpaceFeeds).toBe(false);
    store.observeSpaceHolotank({ ship_id: "ship.1" });
    expect(store.hasSpaceFeeds).toBe(true);
    expect(store.spaceMode).toBe("holotank");
    store.observeSpaceDocking({ session_id: "dock.ship.1" });
    expect(store.spaceMode).toBe("docking");
  });
});
