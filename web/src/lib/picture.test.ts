// The cockpit's picture in the web client (doc 21 cut G.3c): the look's
// swing, the keys, the lock order and the click maths — pure, so pinned here.

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
  dockOrderToFrameInput,
  dockOrderToInputMask,
  gateLine,
  gateWord,
  lockOrder,
  LOOK_MDEG_PER_S,
  nextLock,
  nextPictureKind,
  nextSpaceCamera,
  orderPrefill,
  pickCanvasContact,
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
  ZOOM_DEFAULT_BY_KIND,
} from "./picture";
import { FEED_SPACE_DOCKING, PixelModule } from "./pixel/wasm";
import { PictureStore } from "./stores/picture.svelte";

/** The committed fixture frame's contacts: hostile BA at 450 m, friendly RB
 * at 100 m, unidentified X3 at 200 m and X4 at 600 m; BA is locked. */
function fixtureContacts(): CockpitContact[] {
  const raw: unknown = JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/cockpit/cockpit_frame.json"), "utf8"),
  );
  return decodeCockpitFrame(raw)!.contacts;
}

describe("the look", () => {
  it("wraps a look into -180..180 degrees", () => {
    expect(wrapMdeg(0)).toBe(0);
    expect(wrapMdeg(180_000)).toBe(-180_000);
    expect(wrapMdeg(-190_000)).toBe(170_000);
    expect(wrapMdeg(720_500)).toBe(500);
  });

  it("swings the look at 45 degrees a second, a second at most a step", () => {
    expect(LOOK_MDEG_PER_S).toBe(45_000);
    expect(swingLook(0, 1, 100)).toBe(4_500);
    expect(swingLook(0, -1, 100)).toBe(-4_500);
    expect(swingLook(178_000, 1, 100)).toBe(-177_500);
    expect(swingLook(0, 1, 5_000)).toBe(45_000);
    expect(swingLook(1_000, 0, 100)).toBe(1_000);
  });

  it("reads the keys the TUI's view reads", () => {
    expect(pictureKey("ArrowLeft")).toEqual({ kind: "turn", dir: -1 });
    expect(pictureKey("ArrowRight")).toEqual({ kind: "turn", dir: 1 });
    expect(pictureKey(",")).toEqual({ kind: "look", dir: -1 });
    expect(pictureKey(".")).toEqual({ kind: "look", dir: 1 });
    expect(pictureKey("c")).toEqual({ kind: "centre" });
    expect(pictureKey(" ")).toEqual({ kind: "steady" });
    expect(pictureKey("j")).toBeNull();
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

  it("b is the glass, n the scan, z and P the startup, m EMCON", () => {
    expect(pictureKey("b")).toEqual({ kind: "glass" });
    expect(pictureKey("n")).toEqual({ kind: "scan" });
    expect(pictureKey("z")).toEqual({ kind: "startup" });
    expect(pictureKey("P")).toEqual({ kind: "startup" });
    expect(pictureKey("m")).toEqual({ kind: "emcon" });
  });

  it("l and F arm or disarm the standing rules", () => {
    expect(pictureKey("l")).toEqual({ kind: "autoTarget" });
    expect(pictureKey("F")).toEqual({ kind: "autoFire" });
  });

  it("y and N answer the detachment", () => {
    expect(pictureKey("y")).toEqual({ kind: "wilco" });
    expect(pictureKey("N")).toEqual({ kind: "negative" });
  });

  it("reads a letter's case as part of the key, as the TUI does", () => {
    // A capital that is its own key is a different one from its lower case.
    expect(pictureKey("T")).not.toEqual(pictureKey("t"));
    expect(pictureKey("F")).not.toEqual(pictureKey("f"));
    expect(pictureKey("N")).not.toEqual(pictureKey("n"));
    // A capital with no binding of its own does nothing: it is no alias.
    for (const k of ["A", "D", "W", "S", "X", "Q", "E", "C", "V", "G", "B", "Z", "M", "L", "Y", "p"]) {
      expect(pictureKey(k), k).toBeNull();
    }
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
    expect(gateWord(false)).toBe("weapons HOLD");
    expect(gateWord(true)).toBe("weapons free");
    expect(gateWord(null)).toBe("weapons free");
  });
});

describe("the lock cycle (doc 21 cut G.12c)", () => {
  it("orders hostiles first, then unidentified, then the rest, nearest first in each", () => {
    expect(lockOrder(fixtureContacts())).toEqual(["BA", "X3", "X4", "RB"]);
  });

  it("breaks a tie in range by the designator, and leaves out a contact with none", () => {
    const contacts = fixtureContacts();
    const ba = contacts.find((c) => c.designator === "BA")!;
    const rb = contacts.find((c) => c.designator === "RB")!;
    const x3 = contacts.find((c) => c.designator === "X3")!;
    const at = (designator: string, range_m: number): CockpitContact => ({ ...x3, designator, range_m });
    // The frame's own order is no part of it: the same contacts either way round.
    const one = [at("X9", 200), at("X2", 200), at("", 50), ba, rb];
    const other = [rb, ba, at("", 50), at("X2", 200), at("X9", 200)];
    expect(lockOrder(one)).toEqual(["BA", "X2", "X9", "RB"]);
    expect(lockOrder(other)).toEqual(["BA", "X2", "X9", "RB"]);
    expect(nextLock([at("", 50)], null, 1)).toBeNull();
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
  it("opens each camera at its own zoom, the 2.5D one zoomed out", () => {
    const store = new PictureStore();
    expect(store.zoomByKind).toEqual({ cockpit: 1_000, overhead: 1_000, iso25d: 500 });
    expect(store.spaceZoomPermille).toBe(1_000);
    // One store's zoom is its own: the defaults are copied, never shared.
    store.zoomByKind.overhead = 1_500;
    expect(new PictureStore().zoomByKind.overhead).toBe(1_000);
    expect(ZOOM_DEFAULT_BY_KIND.overhead).toBe(1_000);
  });

  it("keeps the look, the zoom and the ordered speed when the connection goes", () => {
    const store = new PictureStore();
    store.lookMdeg = 30_000;
    store.zoomByKind.iso25d = 750;
    store.throttle.bump(1, 0);
    store.observeSpaceHolotank({ active: true });
    expect(store.spaceMode).toBe("holotank");
    store.clear();
    expect(store.spaceMode).toBeNull();
    expect(store.lookMdeg).toBe(30_000);
    expect(store.zoomByKind.iso25d).toBe(750);
    expect(store.throttle.ordered).toBe(6);
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

  it("steps a camera's zoom inside its own range and resets to its own default", () => {
    expect(pictureKey("+")).toEqual({ kind: "zoom", dir: 1 });
    expect(pictureKey("-")).toEqual({ kind: "zoom", dir: -1 });
    expect(stepZoomPermille(1_000, 1)).toBe(1_250);
    expect(stepZoomPermille(2_000, 1)).toBe(2_000);
    expect(stepZoomPermille(500, -1)).toBe(500);
    // First person never zooms out past 1.0x.
    expect(stepZoomPermille(1_000, -1, "cockpit")).toBe(1_000);
    // The reset is the camera's own opening zoom, the TUI's `zoom_by_camera`.
    expect(stepZoomPermille(1_500, 0, "cockpit")).toBe(1_000);
    expect(stepZoomPermille(1_500, 0, "overhead")).toBe(1_000);
    expect(stepZoomPermille(1_500, 0, "iso25d")).toBe(500);
    expect(stepZoomPermille(1_500, 0)).toBe(500);
  });

  it("picks a contact where the renderer drew it, at either frame size", () => {
    const contacts = fixtureContacts();
    // Friendly RB is 100 m due west (bearing 270). The 2.5D camera at 640 wide
    // and 1.0x draws 1.15 px a metre east: 115 px left of centre.
    expect(pickCanvasContact(contacts, "iso25d", (320 - 115) / 640, 0.5, 1_000)).toBe("RB");
    // At 0.5x it is half as far out, and the old spot is nobody's.
    expect(pickCanvasContact(contacts, "iso25d", (320 - 57.5) / 640, 0.5, 500)).toBe("RB");
    expect(pickCanvasContact(contacts, "iso25d", (320 - 230) / 640, 0.5, 500)).toBeNull();
    // At 1280 wide the scale grows by the root of the width: 1.15 x sqrt(2).
    const hd = 115 * Math.SQRT2;
    expect(pickCanvasContact(contacts, "iso25d", (640 - hd) / 1280, 0.5, 1_000, 600, 0, 1280, 720)).toBe("RB");
    // The overhead's scale is half the frame's short side over the reach
    // (600 m of sight, 690 m of reach): 240/690 px a metre at 640x480...
    expect(pickCanvasContact(contacts, "overhead", (320 - (100 * 240) / 690) / 640, 0.5, 1_000, 600)).toBe("RB");
    // ...and 360/690 at 1280x720: the short side is the height, not the width.
    expect(
      pickCanvasContact(contacts, "overhead", (640 - (100 * 360) / 690) / 1280, 0.5, 1_000, 600, 0, 1280, 720),
    ).toBe("RB");
    // At 2.0x the reach halves: 360/345 px a metre, where half the width over
    // the reach would land 81 px further out, on nobody.
    expect(
      pickCanvasContact(contacts, "overhead", (640 - (100 * 360) / 345) / 1280, 0.5, 2_000, 600, 0, 1280, 720),
    ).toBe("RB");
    expect(
      pickCanvasContact(contacts, "overhead", (640 - (100 * 640) / 345) / 1280, 0.5, 2_000, 600, 0, 1280, 720),
    ).toBeNull();
    // First person looking due west: RB is dead ahead, at the frame's centre.
    expect(pickCanvasContact(contacts, "cockpit", 0.5, 0.5, 1_000, 600, 270)).toBe("RB");
    expect(pickCanvasContact(contacts, "cockpit", 0.5, 0.5, 1_000, 600, 270, 1280, 720)).toBe("RB");
    expect(pickCanvasContact(contacts, "cockpit", 90 / 640, 0.5, 1_000)).toBeNull();
  });

  it("reads a double click's heading from the frame's own shape", () => {
    // Straight north of centre is 0 degrees, straight east 90.
    expect(canvasClickHeadingDeg("iso25d", 0.5, 0.2)).toBe(0);
    expect(canvasClickHeadingDeg("iso25d", 0.8, 0.5)).toBe(90);
    expect(canvasClickHeadingDeg("iso25d", 0.5, 0.5)).toBeNull();
    expect(canvasClickHeadingDeg("cockpit", 0.8, 0.5)).toBeNull();
    // The overhead is square on the ground: 100 px east and 100 px north is
    // north-east in a 1280x720 frame, which 640x480 arithmetic read as 037.
    expect(canvasClickHeadingDeg("overhead", (640 + 100) / 1280, (360 - 100) / 720, 1280, 720)).toBe(45);
    expect(canvasClickHeadingDeg("overhead", (320 + 100) / 640, (240 - 100) / 480)).toBe(45);
    // The deadzone grows with the frame: 30 px is outside it at 640, inside at 1280.
    expect(canvasClickHeadingDeg("overhead", (320 + 30) / 640, 0.5)).toBe(90);
    expect(canvasClickHeadingDeg("overhead", (640 + 30) / 1280, 0.5, 1280, 720)).toBeNull();
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

    // Holo-Tank orbit & Space Canopy camera + tactical fire keys (`SF.R13`, matching TUI `pixel.rs`).
    expect(spacePictureKey("a", "holotank")).toEqual({
      kind: "holotankOrbit",
      deltaMdeg: -15_000,
    });
    expect(spacePictureKey("d", "holotank")).toEqual({
      kind: "holotankOrbit",
      deltaMdeg: 15_000,
    });
    expect(spacePictureKey("z", "holotank")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/beam",
    });
    expect(spacePictureKey("x", "holotank")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/torp",
    });
    expect(spacePictureKey("c", "holotank")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/railgun",
    });
    expect(spacePictureKey("r", "holotank")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/scan",
    });
    expect(spacePictureKey("q", "space_cockpit")).toEqual({
      kind: "look",
      dir: -1,
    });
    expect(spacePictureKey("z", "space_cockpit")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/beam",
    });
    expect(spacePictureKey("x", "space_cockpit")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/torp",
    });
    expect(spacePictureKey("c", "space_cockpit")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/railgun",
    });
    expect(spacePictureKey("r", "space_cockpit")).toEqual({
      kind: "spaceOrder",
      cmd: "+tac/scan",
    });
    expect(spacePictureKey("w", "space_cockpit")).toBeNull();
    expect(spacePictureKey("g", "space_cockpit")).toBeNull();
    // A letter's case is the TUI's: the holo-tank's and the canopy's are lower
    // case only, the docking camera's read either.
    for (const k of ["A", "D", "Z", "X", "C", "R", "V"]) {
      expect(spacePictureKey(k, "holotank"), k).toBeNull();
    }
    for (const k of ["Q", "E", "Z", "X", "C", "R", "V"]) {
      expect(spacePictureKey(k, "space_cockpit"), k).toBeNull();
    }
    expect(spacePictureKey("V", "docking")).toBeNull();
    expect(spacePictureKey("W", "docking")).toEqual(spacePictureKey("w", "docking"));
    expect(spacePictureKey("v", "docking")).toEqual({ kind: "spaceView" });

    // PictureStore space feed observers auto-select spaceMode.
    const store = new PictureStore();
    expect(store.hasSpaceFeeds).toBe(false);
    store.observeSpaceHolotank({ ship_id: "ship.1" });
    expect(store.hasSpaceFeeds).toBe(true);
    expect(store.spaceMode).toBe("holotank");
    store.observeSpaceDocking({ session_id: "dock.ship.1" });
    expect(store.spaceMode).toBe("docking");
  });

  it("plays the 60 Hz web client docking game in mush-pixel-wasm via player keys and formats +helm/dock tape <session_id> <rle>", () => {
    const wasmBytes = readFileSync(resolve(process.cwd(), "src/lib/pixel/mush_pixel.wasm"));
    const mod = new PixelModule(new WebAssembly.Instance(new WebAssembly.Module(wasmBytes)));
    const feed = {
      session_id: "dock-42-1",
      dx_mm: -480,
      dy_mm: -420,
      dz_mm: 2400,
      vx_mmps: 0,
      vy_mmps: 0,
      vz_mmps: 0,
      pitch_mdeg: -2880,
      yaw_mdeg: 2520,
      roll_mdeg: -3840,
      pitch_rate_mdeg_s: 0,
      yaw_rate_mdeg_s: 0,
      roll_rate_mdeg_s: 0,
      dampeners: true,
      target_tumble_mdeg_s: [0, 0, 0],
      rcs_authority: {
        trans_dv_mmps_per_frame: 32,
        closure_dv_mmps_per_frame: 28,
        attitude_rate_mdeg_s_per_frame: 195,
        roll_rate_mdeg_s_per_frame: 260,
        damp_trans_mmps_per_frame: 16,
        damp_attitude_mdeg_s_per_frame: 95,
        damp_roll_mdeg_s_per_frame: 130,
        kg_per_pulse_batch: 1,
      },
    };
    expect(mod.feed(FEED_SPACE_DOCKING, JSON.stringify(feed))).toBe(true);
    const initial = mod.dockingState();
    expect(initial).not.toBeNull();
    expect(initial?.session_id).toBe("dock-42-1");

    // `c`, `C`, and Space are NOT bound to any auto-solver (`SF.R24`):
    expect(spacePictureKey("c", "docking")).toBeNull();
    expect(spacePictureKey("C", "docking")).toBeNull();
    expect(spacePictureKey(" ", "docking")).toBeNull();

    const rightKey = spacePictureKey("d", "docking");
    expect(rightKey).toEqual({ kind: "dockOrder", cmd: "DOCK TRANS RIGHT" });
    expect(dockOrderToFrameInput("DOCK TRANS RIGHT")?.transX).toBe(1);
    const rightMask = dockOrderToInputMask("DOCK TRANS RIGHT");
    expect(rightMask).toBe(1);
    const afterOne = mod.stepDocking(rightMask ?? 0);
    expect(afterOne?.feed.vx_mmps).toBe(32);
    expect(afterOne?.clamped).toBe(false);

    // Step the WASM session to clamp using real player keys and coast frames:
    const pressKey = (key: string): number => {
      const k = spacePictureKey(key, "docking");
      if (!k || k.kind !== "dockOrder") return 0;
      return dockOrderToInputMask(k.cmd) ?? 0;
    };
    let st = afterOne;
    for (let i = 0; i < 1200 && st && !st.clamped; i += 1) {
      const f = st.feed;
      const dx = Number(f.dx_mm ?? 0);
      const dy = Number(f.dy_mm ?? 0);
      const vx = Number(f.vx_mmps ?? 0);
      const vy = Number(f.vy_mmps ?? 0);
      const vz = Number(f.vz_mmps ?? 0);
      const pitch = Number(f.pitch_mdeg ?? 0);
      const yaw = Number(f.yaw_mdeg ?? 0);
      const roll = Number(f.roll_mdeg ?? 0);
      const pitchRate = Number(f.pitch_rate_mdeg_s ?? 0);
      const yawRate = Number(f.yaw_rate_mdeg_s ?? 0);
      const rollRate = Number(f.roll_rate_mdeg_s ?? 0);
      let mask = 0;
      if (dx < -80 && vx < 160) {
        mask = pressKey("d");
      } else if (dx > 80 && vx > -160) {
        mask = pressKey("a");
      } else if (dy < -80 && vy < 160) {
        mask = pressKey("w");
      } else if (dy > 80 && vy > -160) {
        mask = pressKey("s");
      } else if (pitch < -500 && pitchRate < 350) {
        mask = pressKey("ArrowUp");
      } else if (pitch > 500 && pitchRate > -350) {
        mask = pressKey("ArrowDown");
      } else if (yaw > 500 && yawRate > -350) {
        mask = pressKey("ArrowLeft");
      } else if (yaw < -500 && yawRate < 350) {
        mask = pressKey("ArrowRight");
      } else if (roll < -600 && rollRate < 450) {
        mask = pressKey("e");
      } else if (roll > 600 && rollRate > -450) {
        mask = pressKey("q");
      } else if (
        Math.abs(dx) <= 250 &&
        Math.abs(dy) <= 250 &&
        Math.abs(pitch) <= 2000 &&
        Math.abs(yaw) <= 2000 &&
        Math.abs(roll) <= 3000 &&
        vx === 0 &&
        vy === 0 &&
        pitchRate === 0 &&
        yawRate === 0 &&
        rollRate === 0 &&
        vz < 300
      ) {
        mask = pressKey("r");
      }
      st = mod.stepDocking(mask);
    }

    expect(st?.clamped).toBe(true);
    expect(st?.gates_green).toBe(5);
    expect(st?.feed.status).toBe("ALIGNED-READY");
    expect(st?.tape_command.startsWith("+helm/dock tape dock-42-1 ")).toBe(true);
  });
});
