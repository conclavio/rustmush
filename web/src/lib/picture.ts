// The cockpit's picture in the web client (doc 21 cuts G.3c, G.8): the page
// draws it from this connection's own cockpit and map feeds, with the same
// renderer the SSH TUI runs (`mush-pixel`, compiled to wasm). Everything here
// is pure (the look's swing, the keys, the speed's steps, the lock order, the
// click maths), so vitest pins it without a socket or a canvas.
//
// The look is the camera's offset from the tank's heading and is the
// viewer's alone: the world never hears of it. Turning the tank is a
// `HEADING` line like any typed order, which the picture's turn keys send
// exactly as the SSH TUI's do (G.4b, `./helm`): the one place the cockpit tab
// sends an order rather than prefilling one.
//
// Doc 21 cut G.12c binds section 7.3's flight keys, the same keys the TUI's
// screen 0 reads and in the same case: `a`/`d` turn as the arrows do, `w`/`s`
// step the ordered speed, `x` all stop, `q`/`e` look, `f` the gate, `g` the
// volley, `t`/`T` the lock cycle, `v` the camera, `b` the glass, `n` the
// scan, `z` the startup, `m` EMCON, `l` and `F` the standing rules, `y`/`N`
// the detachment's answer, and `F1`-`F6` the `ORDER` prefills (the one
// family that still prefills rather than sends). The stepping, the clamp and
// the lock order are pure here so the TUI's rules and the web's are pinned
// side by side.

import type { CockpitContact } from "./cockpit";

/** How fast a held look swings, millidegrees a second (the TUI's 45°/s). */
export const LOOK_MDEG_PER_S = 45_000;

/** How often a swinging look is sent again, milliseconds (10 Hz). */
export const LOOK_STEP_MS = 100;

/** One full turn in millidegrees. */
export const FULL_MDEG = 360_000;

/** Wrap millidegrees into `0..360_000`. */
export function wrap360Mdeg(mdeg: number): number {
  const m = Math.round(mdeg);
  return ((m % FULL_MDEG) + FULL_MDEG) % FULL_MDEG;
}

/** Shortest signed arc `to - from` in `-180_000..=180_000` millidegrees (`helm::arc`). */
export function arcMdeg(from: number, to: number): number {
  const d = wrap360Mdeg(to - from);
  return d > FULL_MDEG / 2 ? d - FULL_MDEG : d;
}

/** Dead-reckoned `(dxCm, dyCm)` position offset from the last keyframe's `(x_cm, y_cm)`
 * integrated along `headingMdeg` for `elapsedMs` (clamped to `maxElapsedMs`). */
export function predictedOffsetCm(
  speedCms: number,
  headingMdeg: number,
  elapsedMs: number,
  maxElapsedMs = 1_500,
): { dxCm: number; dyCm: number } {
  if (!Number.isFinite(speedCms) || speedCms === 0 || elapsedMs <= 0) {
    return { dxCm: 0, dyCm: 0 };
  }
  const dtMs = Math.min(Math.max(0, elapsedMs), maxElapsedMs);
  const distCm = (speedCms * dtMs) / 1_000;
  const rad = ((wrap360Mdeg(headingMdeg) / 1_000) * Math.PI) / 180;
  return {
    dxCm: Math.round(distCm * Math.sin(rad)) | 0,
    dyCm: Math.round(distCm * Math.cos(rad)) | 0,
  };
}

/** Whether this viewer last wanted the picture (a per-viewer convenience). */
export const PICTURE_PREF_KEY = "rustmush.picture";

/** Which camera the picture is (doc 21 §7.4): the first-person view, the
 * overhead (north-up with the pilot's tank at its centre), or the oblique
 * 2.5D tactical camera. */
export type PictureKind = "cockpit" | "overhead" | "iso25d";

/** Cycle to the next cockpit camera (`cockpit` → `overhead` → `iso25d` → `cockpit`),
 * matching the SSH TUI's `v` key. */
export function nextPictureKind(kind: PictureKind): PictureKind {
  switch (kind) {
    case "cockpit":
      return "overhead";
    case "overhead":
      return "iso25d";
    case "iso25d":
      return "cockpit";
  }
}

/** A look in `-180_000..180_000` millidegrees. */
export function wrapMdeg(mdeg: number): number {
  const m = Math.round(mdeg);
  return ((((m + 180_000) % 360_000) + 360_000) % 360_000) - 180_000;
}

/** The look after `ms` more of a swing in `dir` (`-1` left, `1` right). */
export function swingLook(lookMdeg: number, dir: -1 | 0 | 1, ms: number): number {
  return wrapMdeg(lookMdeg + (dir * LOOK_MDEG_PER_S * Math.min(ms, 1_000)) / 1_000);
}

/** An `ORDER` prefill's addressee: every slot, or one of slots 2–6 (slot 1 is
 * the pilot's own, doc 26's detachment). */
export type OrderSlot = "ALL" | 2 | 3 | 4 | 5 | 6;

/** What one key does to the picture, or `null` for a key it leaves alone. */
export type PictureKey =
  /** The tank turns, held: keydown starts it, keyup stops it. */
  | { kind: "turn"; dir: -1 | 1 }
  /** The look swings, toggled (again to stop). */
  | { kind: "look"; dir: -1 | 1 }
  | { kind: "centre" }
  /** Space: the turn and the look stop; the speed stands. */
  | { kind: "steady" }
  /** One speed step up or down (`SPEED`). */
  | { kind: "speed"; dir: -1 | 1 }
  /** All stop: `SPEED 0`, and a held turn let go; a `HEADING` ordered and
   * not yet reached stands. */
  | { kind: "allStop" }
  /** The next camera: first person, overhead, 2.5D. */
  | { kind: "view" }
  /** Weapons free ↔ weapons hold (`FIRE FREE` / `FIRE HOLD`). */
  | { kind: "gate" }
  /** One volley (`FIRE ALL`). */
  | { kind: "volley" }
  /** Fire an individual weapon slot `0..=3` (`Shift+1`..`Shift+4` -> `!`, `@`, `#`, `$`). */
  | { kind: "fireSlot"; slot: 0 | 1 | 2 | 3 }
  /** The next (`1`) or previous (`-1`) contact locked (`LOCK`). */
  | { kind: "lock"; dir: -1 | 1 }
  /** The canopy glass on or off (first person; the viewer's alone). */
  | { kind: "glass" }
  /** `SCAN` the lock. */
  | { kind: "scan" }
  /** `STARTUP`. */
  | { kind: "startup" }
  /** `EMCON on` / `EMCON off`, on a unit that carries the pod. */
  | { kind: "emcon" }
  /** The standing lock rule armed or disarmed (`LOCK nearest WHEN nolock`). */
  | { kind: "autoTarget" }
  /** The standing fire rule armed or disarmed (`FIRE ALL WHEN odds >= 60`). */
  | { kind: "autoFire" }
  /** The detachment's proposal accepted (its `wilco_command`). */
  | { kind: "wilco" }
  /** The detachment's proposal refused (its `negative_command`). */
  | { kind: "negative" }
  /** Zoom in (`1`) or out (`-1`) on the active cockpit camera. */
  | { kind: "zoom"; dir: -1 | 1 }
  /** The chat line opened with an `ORDER` ready. */
  | { kind: "order"; slot: OrderSlot };

/**
 * The picture's keys (doc 21 §7.3, cut G.12c): the TUI's screen-0 keys.
 *
 * A letter's case is part of the key, as on the TUI: `t` is the next lock and
 * `T` the previous, `f` the gate and `F` the standing fire rule, `n` the scan
 * and `N` the detachment's negative. The caller drops a key chorded with
 * Ctrl, Alt or Meta: those belong to the browser.
 */
export function pictureKey(key: string): PictureKey | null {
  switch (key) {
    case "ArrowLeft":
    case "a":
      return { kind: "turn", dir: -1 };
    case "ArrowRight":
    case "d":
      return { kind: "turn", dir: 1 };
    case "w":
      return { kind: "speed", dir: 1 };
    case "s":
      return { kind: "speed", dir: -1 };
    case "x":
      return { kind: "allStop" };
    case ",":
    case "q":
      return { kind: "look", dir: -1 };
    case ".":
    case "e":
      return { kind: "look", dir: 1 };
    case "c":
      return { kind: "centre" };
    case " ":
      return { kind: "steady" };
    case "v":
      return { kind: "view" };
    case "f":
      return { kind: "gate" };
    case "F":
      return { kind: "autoFire" };
    case "g":
      return { kind: "volley" };
    case "b":
      return { kind: "glass" };
    case "n":
      return { kind: "scan" };
    case "N":
      return { kind: "negative" };
    case "y":
      return { kind: "wilco" };
    case "z":
    case "P":
      return { kind: "startup" };
    case "m":
      return { kind: "emcon" };
    case "l":
      return { kind: "autoTarget" };
    case "!":
      return { kind: "fireSlot", slot: 0 };
    case "@":
      return { kind: "fireSlot", slot: 1 };
    case "#":
      return { kind: "fireSlot", slot: 2 };
    case "$":
      return { kind: "fireSlot", slot: 3 };
    case "t":
      return { kind: "lock", dir: 1 };
    case "T":
      return { kind: "lock", dir: -1 };
    case "+":
    case "=":
      return { kind: "zoom", dir: 1 };
    case "-":
    case "_":
      return { kind: "zoom", dir: -1 };
    case "F1":
      return { kind: "order", slot: "ALL" };
    case "F2":
      return { kind: "order", slot: 2 };
    case "F3":
      return { kind: "order", slot: 3 };
    case "F4":
      return { kind: "order", slot: 4 };
    case "F5":
      return { kind: "order", slot: 5 };
    case "F6":
      return { kind: "order", slot: 6 };
    default:
      return null;
  }
}

/** The line an `order` key opens the chat line with — `ORDER ALL ` or
 * `ORDER 2 ` … `ORDER 6 ` — for the pilot to finish; Enter sends it. */
export function orderPrefill(slot: OrderSlot): string {
  return `ORDER ${slot} `;
}

// -- the speed (doc 21 §7.3 `w` / `s`) ----------------------------------------

/** The `SPEED` ceilings a frame from before G.12a is stepped against. */
export const DEFAULT_MAX_SPEED_KPH = 60;
export const DEFAULT_REVERSE_KPH = 20;

/** The least time between two speed orders, milliseconds (the TUI's order
 * gap). The first step goes at once; the steps inside the gap coalesce and
 * the last value goes when it opens, so a held `w` sends four orders a
 * second rather than one a key repeat (`./helm`'s `Throttle`). */
export const SPEED_COALESCE_MS = 250;

/** Within how many kph an ordered speed counts as reached. */
export const SPEED_REACHED_KPH = 1;

/** One speed step: a tenth of the hull's top speed, at least 5 kph. */
export function speedStep(maxKph: number): number {
  return Math.max(5, Math.round(maxKph / 10));
}

/**
 * The speed one step `dir` from `fromKph`, through zero into reverse,
 * clamped to `[-reverseKph, maxKph]`: past either ceiling the step clamps
 * rather than being refused.
 */
export function steppedSpeed(fromKph: number, dir: -1 | 1, maxKph: number, reverseKph: number): number {
  const next = Math.round(fromKph) + dir * speedStep(maxKph);
  return Math.min(maxKph, Math.max(-reverseKph, next));
}

/** Whether an ordered speed is still outstanding — not yet reached within
 * [`SPEED_REACHED_KPH`]. While it is, the next step goes from it and the
 * header shows it beside the frame's. */
export function speedOutstanding(ordered: number | null, frameKph: number): ordered is number {
  return ordered !== null && Math.abs(ordered - frameKph) > SPEED_REACHED_KPH;
}

/** The `SPEED` line. `QUIET`, as the helm's `HEADING`: a key's order is taken
 * without the `Making … kph.` a typed one is acknowledged with (doc 21 §7.3). */
export function speedLine(kph: number): string {
  return `SPEED ${kph} QUIET`;
}

// -- the weapons gate (doc 21 §7.3 `f`) ----------------------------------------

/** The gate's line: `FIRE FREE` from hold, `FIRE HOLD` from free. A frame
 * without the key (`null`, a server from before G.12a) reads as free — the
 * seat's default (Q-21-9) and the behaviour before the gate existed. */
export function gateLine(weaponsFree: boolean | null): string {
  return weaponsFree === false ? "FIRE FREE" : "FIRE HOLD";
}

/** The header's word for the gate, as the TUI's top line prints it. */
export function gateWord(weaponsFree: boolean | null): string {
  return weaponsFree === false ? "weapons HOLD" : "weapons free";
}

// -- the lock cycle (doc 21 §7.3 `t` / `T`) ------------------------------------

/** How long the lock a key last asked for stands in for the frame's, so a
 * second `t` before the next frame walks on (a pulse or two at the arena's
 * tempo); past it, or once the frame shows it, the frame's lock is the one. */
export const LOCK_PENDING_MS = 1_500;

/** A contact's band in the lock cycle, by the viewer's relation (doc 32 cut
 * CF.0): hostiles, then unidentified, then neutrals, then own and friends —
 * the TUI's `lock_order`. */
function lockRank(c: CockpitContact): number {
  switch (c.relation) {
    case "hostile":
      return 0;
    case "neutral":
      return 2;
    case "own":
    case "friend":
      return 3;
    default:
      return 1;
  }
}

/** The contacts in the lock cycle's order: hostiles first, then unidentified,
 * then the rest, nearest first within each and the designator breaking a
 * tie. A contact with no designator cannot be locked and is left out. */
export function lockOrder(contacts: readonly CockpitContact[]): string[] {
  return contacts
    .filter((c) => c.designator !== "")
    .sort(
      (a, b) =>
        lockRank(a) - lockRank(b) ||
        a.range_m - b.range_m ||
        (a.designator < b.designator ? -1 : a.designator > b.designator ? 1 : 0),
    )
    .map((c) => c.designator);
}

/**
 * The designator `t` (`dir` 1) or `T` (`-1`) locks: the next or previous in
 * [`lockOrder`] from `current`, wrapping; from no lock — or a lock no longer
 * listed — the first, either way (doc 21 §7.3); `null` with no contacts.
 */
export function nextLock(
  contacts: readonly CockpitContact[],
  current: string | null,
  dir: -1 | 1,
): string | null {
  const order = lockOrder(contacts);
  if (order.length === 0) return null;
  const at = current === null ? -1 : order.indexOf(current);
  if (at < 0) return order[0] ?? null;
  return order[(at + dir + order.length) % order.length] ?? null;
}

// -- 2.5D zoom and interactive tactical canvas (`iso25d` / `overhead`) ---------

export const ZOOM_MIN_PERMILLE = 500;
export const ZOOM_MAX_PERMILLE = 2_000;
export const ZOOM_DEFAULT_PERMILLE = 1_000;
export const ZOOM_STEP_PERMILLE = 250;

/** The zoom each camera opens at, and goes back to on a reset: the TUI's
 * `zoom_by_camera`. The 2.5D camera opens zoomed out, where the pilot would
 * otherwise press `-` twice to see the field. */
export const ZOOM_DEFAULT_BY_KIND: Readonly<Record<PictureKind, number>> = {
  cockpit: 1_000,
  overhead: 1_000,
  iso25d: 500,
};

/** Step the camera zoom in permille (`1000..=2000` in first-person `cockpit`, `500..=2000`
 * in `overhead` / `iso25d`, `1000` = 1.0x), or go back to the camera's own default on `0`. */
export function stepZoomPermille(
  current: number,
  dir: -1 | 0 | 1,
  kind: PictureKind = "iso25d",
): number {
  if (dir === 0) return ZOOM_DEFAULT_BY_KIND[kind];
  const minZoom = kind === "cockpit" ? ZOOM_DEFAULT_PERMILLE : ZOOM_MIN_PERMILLE;
  const base =
    Number.isFinite(current) && current > 0
      ? Math.max(minZoom, Math.round(current))
      : ZOOM_DEFAULT_BY_KIND[kind];
  return Math.min(ZOOM_MAX_PERMILLE, Math.max(minZoom, base + dir * ZOOM_STEP_PERMILLE));
}

/** The frame the click maths assumes when the caller names none: the SD one. */
const PICK_FRAME_W = 640;
const PICK_FRAME_H = 480;

/** A zoom in permille as the ground cameras take it: `0.5..=2.0`. */
function groundZoom(zoomPermille: number): number {
  return (
    Math.min(
      ZOOM_MAX_PERMILLE,
      Math.max(ZOOM_MIN_PERMILLE, zoomPermille > 0 ? zoomPermille : ZOOM_DEFAULT_PERMILLE),
    ) / 1_000
  );
}

/** The 2.5D camera's ground scale at a frame width and zoom, pixels a metre
 * east and north: `mush_pixel::iso25d`'s `IsoProj::for_viewport_zoomed`. */
function isoPxPerM(frameW: number, zoomPermille: number): { x: number; y: number } {
  const s =
    Math.min(1.45, Math.max(0.7, Math.sqrt(frameW / PICK_FRAME_W))) * groundZoom(zoomPermille);
  return { x: 1.15 * s, y: 0.78 * s };
}

/** The overhead camera's scale, pixels a metre: half the frame's short side
 * over the reach (`mush_pixel::overhead`'s `effective_reach`). */
function overheadPxPerM(
  frameW: number,
  frameH: number,
  zoomPermille: number,
  sightM: number,
): number {
  const baseReach = Math.min(3_000, Math.max(150, (sightM > 0 ? sightM : 600) * 1.15));
  const reach = Math.min(6_000, Math.max(75, baseReach / groundZoom(zoomPermille)));
  return Math.min(frameW, frameH) / 2 / reach;
}

/** Pick the nearest contact on the `cockpit`, `overhead`, or `iso25d` canvas at normalized
 * `[0, 1]` coordinates `(normX, normY)`, returning its `designator` when within the hit radius.
 * `frameW` x `frameH` is the frame the renderer drew (640x480, or 1280x720 in HD): the 2.5D
 * and overhead scales depend on it, and the hit radius grows with it. */
export function pickCanvasContact(
  contacts: readonly CockpitContact[],
  kind: PictureKind,
  normX: number,
  normY: number,
  zoomPermille = ZOOM_DEFAULT_PERMILLE,
  sightM = 600,
  cameraHeadingDeg = 0,
  frameW = PICK_FRAME_W,
  frameH = PICK_FRAME_H,
): string | null {
  if (!Number.isFinite(normX) || !Number.isFinite(normY)) return null;
  const px = normX * frameW;
  const py = normY * frameH;
  const halfW = frameW / 2;
  const halfH = frameH / 2;
  // A hit radius set at 640 wide, in the frame's own pixels.
  const grow = frameW / PICK_FRAME_W;
  if (kind === "cockpit") {
    const z =
      Math.min(
        ZOOM_MAX_PERMILLE,
        Math.max(ZOOM_DEFAULT_PERMILLE, zoomPermille > 0 ? zoomPermille : ZOOM_DEFAULT_PERMILLE),
      ) / 1_000;
    let bestDesig: string | null = null;
    let bestDistSq = (44 * grow) ** 2;
    for (const c of contacts) {
      if (!c.designator) continue;
      const relDeg = ((((c.bearing_deg - cameraHeadingDeg + 180) % 360) + 360) % 360) - 180;
      if (Math.abs(relDeg) > 36) continue;
      const rad = (relDeg * Math.PI) / 180;
      const depth = c.range_m * Math.cos(rad);
      if (depth <= 1) continue;
      const planeX = (c.range_m * Math.sin(rad)) / (depth * 0.66);
      const cx = halfW + planeX * halfW * z;
      const cy = halfH;
      if (cx < -32 * grow || cx > frameW + 32 * grow) continue;
      const dx = px - cx;
      const dy = (py - cy) * 0.65;
      const distSq = dx * dx + dy * dy;
      if (distSq <= bestDistSq) {
        bestDistSq = distSq;
        bestDesig = c.designator;
      }
    }
    return bestDesig;
  }
  let pxPerMX: number;
  let pxPerMY: number;
  if (kind === "iso25d") {
    const iso = isoPxPerM(frameW, zoomPermille);
    pxPerMX = iso.x;
    pxPerMY = iso.y;
  } else {
    const pxPerM = overheadPxPerM(frameW, frameH, zoomPermille, sightM);
    pxPerMX = pxPerM;
    pxPerMY = pxPerM;
  }

  let bestDesig: string | null = null;
  let bestDistSq = (34 * grow) ** 2;
  for (const c of contacts) {
    if (!c.designator) continue;
    const rad = (c.bearing_deg * Math.PI) / 180;
    const cx = halfW + c.range_m * Math.sin(rad) * pxPerMX;
    const cy = halfH - c.range_m * Math.cos(rad) * pxPerMY;
    const dx = px - cx;
    const dy = py - cy;
    const distSq = dx * dx + dy * dy;
    if (distSq <= bestDistSq) {
      bestDistSq = distSq;
      bestDesig = c.designator;
    }
  }
  return bestDesig;
}

/** Compass bearing (`0..359` degrees clockwise from north) from the ego unit at canvas
 * centre `(0.5, 0.5)` to `(normX, normY)` in `iso25d` or `overhead` view, or `null` inside
 * the centre deadzone. `frameW` x `frameH` is the frame the renderer drew. */
export function canvasClickHeadingDeg(
  kind: PictureKind,
  normX: number,
  normY: number,
  frameW = PICK_FRAME_W,
  frameH = PICK_FRAME_H,
): number | null {
  if (kind !== "iso25d" && kind !== "overhead") return null;
  if (!Number.isFinite(normX) || !Number.isFinite(normY)) return null;
  const dxPx = (normX - 0.5) * frameW;
  const dyPx = (0.5 - normY) * frameH;
  const dead = (18 * frameW) / PICK_FRAME_W;
  if (dxPx * dxPx + dyPx * dyPx < dead * dead) return null;
  // The 2.5D ground is squashed north to south; the overhead is square.
  const east = kind === "iso25d" ? dxPx / 1.15 : dxPx;
  const north = kind === "iso25d" ? dyPx / 0.78 : dyPx;
  const deg = Math.round((Math.atan2(east, north) * 180) / Math.PI);
  return ((deg % 360) + 360) % 360;
}

// -- Space Bridge & Out-of-Time-Loop 6-DOF Docking (`doc 35` cuts SF.7, SF.8, SF.18) --

/** Panel feed IDs for 3D spaceflight (`crates/tacsim/src/space/commands.rs`). */
export const SPACE_HOLOTANK_FEED = "space.holotank";
export const SPACE_COCKPIT_FEED = "space.cockpit";
export const SPACE_DOCKING_FEED = "space.docking";

/** Space camera mode rendered by `mush-pixel-wasm` (`KIND_HOLOTANK`, `KIND_SPACE_COCKPIT`, `KIND_DOCKING`). */
export type SpaceCameraMode = "holotank" | "space_cockpit" | "docking";

/** Cycle to the next space camera (`holotank` → `space_cockpit` → `docking` → `holotank`). */
export function nextSpaceCamera(mode: SpaceCameraMode): SpaceCameraMode {
  switch (mode) {
    case "holotank":
      return "space_cockpit";
    case "space_cockpit":
      return "docking";
    case "docking":
      return "holotank";
  }
}

/** What one key does while a space camera (`holotank`, `space_cockpit`, or `docking`) is active (`SF.18`). */
export type SpacePictureKey =
  /** Cycle the active space view (`holotank` → `space_cockpit` → `docking`). */
  | { kind: "spaceView" }
  /** Rotate the 3D Holo-Tank azimuth by `deltaMdeg` (`±15_000` mdeg). */
  | { kind: "holotankOrbit"; deltaMdeg: number }
  /** Send an out-of-time-loop 6-DOF docking order (`D-35-16`). */
  | { kind: "dockOrder"; cmd: string }
  /** Send a space tactical weapon or scan order (`+tac/beam`, `+tac/torp`, `+tac/railgun`, `+tac/scan`, `SF.R13`). */
  | { kind: "spaceOrder"; cmd: string }
  /** Swing the canopy look (`-1` left, `1` right). */
  | { kind: "look"; dir: -1 | 1 }
  /** Centre the look (`0` mdeg). */
  | { kind: "centre" }
  /** Step camera zoom (`-1` out, `1` in). */
  | { kind: "zoom"; dir: -1 | 1 };

/**
 * Map a keyboard key to its space camera action (`doc 35` cut `SF.18`, `D-35-16`),
 * matching `crates/mush-net-ssh/src/tui/pixel.rs`.
 *
 * Digits `0`–`9` return `null` so global screen switching (`D-35-16`) is preserved.
 * A letter's case is the TUI's: `v`, the holo-tank's and the canopy's letters are lower
 * case only; the docking camera's read either case, as its keys do there.
 */
export function spacePictureKey(key: string, mode: SpaceCameraMode): SpacePictureKey | null {
  if (key === "v") {
    return { kind: "spaceView" };
  }
  if (key === "+" || key === "=") {
    return { kind: "zoom", dir: 1 };
  }
  if (key === "-" || key === "_") {
    return { kind: "zoom", dir: -1 };
  }
  if (mode === "docking") {
    switch (key) {
      case "w":
      case "W":
        return { kind: "dockOrder", cmd: "DOCK TRANS UP" };
      case "s":
      case "S":
        return { kind: "dockOrder", cmd: "DOCK TRANS DOWN" };
      case "a":
      case "A":
        return { kind: "dockOrder", cmd: "DOCK TRANS LEFT" };
      case "d":
      case "D":
        return { kind: "dockOrder", cmd: "DOCK TRANS RIGHT" };
      case "ArrowUp":
        return { kind: "dockOrder", cmd: "DOCK PITCH UP" };
      case "ArrowDown":
        return { kind: "dockOrder", cmd: "DOCK PITCH DOWN" };
      case "ArrowLeft":
        return { kind: "dockOrder", cmd: "DOCK YAW LEFT" };
      case "ArrowRight":
        return { kind: "dockOrder", cmd: "DOCK YAW RIGHT" };
      case "q":
      case "Q":
        return { kind: "dockOrder", cmd: "DOCK ROLL CCW" };
      case "e":
      case "E":
        return { kind: "dockOrder", cmd: "DOCK ROLL CW" };
      case "r":
      case "R":
        return { kind: "dockOrder", cmd: "DOCK CLOSURE FORWARD" };
      case "f":
      case "F":
        return { kind: "dockOrder", cmd: "DOCK CLOSURE REVERSE" };
      case "x":
      case "X":
        return { kind: "dockOrder", cmd: "DOCK DAMPENERS TOGGLE" };
      default:
        return null;
    }
  }
  if (mode === "holotank") {
    switch (key) {
      case ",":
      case "a":
      case "ArrowLeft":
        return { kind: "holotankOrbit", deltaMdeg: -15_000 };
      case ".":
      case "d":
      case "ArrowRight":
        return { kind: "holotankOrbit", deltaMdeg: 15_000 };
      case "z":
      case "F1":
        return { kind: "spaceOrder", cmd: "+tac/beam" };
      case "x":
      case "F2":
        return { kind: "spaceOrder", cmd: "+tac/torp" };
      case "c":
      case "F3":
        return { kind: "spaceOrder", cmd: "+tac/railgun" };
      case "r":
        return { kind: "spaceOrder", cmd: "+tac/scan" };
      default:
        return null;
    }
  }
  // `space_cockpit`: camera-local look and `+tac/*` weapon group fire keys (`SF.R13`, matching TUI `pixel.rs`).
  switch (key) {
    case ",":
    case "q":
      return { kind: "look", dir: -1 };
    case ".":
    case "e":
      return { kind: "look", dir: 1 };
    case "z":
    case "F1":
      return { kind: "spaceOrder", cmd: "+tac/beam" };
    case "x":
    case "F2":
      return { kind: "spaceOrder", cmd: "+tac/torp" };
    case "c":
    case "F3":
      return { kind: "spaceOrder", cmd: "+tac/railgun" };
    case "r":
      return { kind: "spaceOrder", cmd: "+tac/scan" };
    default:
      return null;
  }
}

/** Per-frame 60 Hz client-side 6-DOF docking input (`SF.R7`, `D-35-14`). */
export interface DockingFrameInput {
  transX: -1 | 0 | 1;
  transY: -1 | 0 | 1;
  closurePulse: -1 | 0 | 1;
  pitchTrim: -1 | 0 | 1;
  yawTrim: -1 | 0 | 1;
  rollTrim: -1 | 0 | 1;
  toggleDampeners?: boolean;
  abort?: boolean;
}

/** Encode a single 60 Hz `DockingFrameInput` into its 16-bit RLE bitmask (`D-35-14`). */
export function encodeDockingInputMask(input: DockingFrameInput): number {
  const axis = (v: -1 | 0 | 1): number => (v > 0 ? 1 : v < 0 ? 2 : 0);
  let mask =
    axis(input.transX) |
    (axis(input.transY) << 2) |
    (axis(input.closurePulse) << 4) |
    (axis(input.pitchTrim) << 6) |
    (axis(input.yawTrim) << 8) |
    (axis(input.rollTrim) << 10);
  if (input.toggleDampeners) mask |= 1 << 12;
  if (input.abort) mask |= 1 << 13;
  return mask;
}

/** Encode a sequence of 60 Hz `DockingFrameInput`s into the bounded RLE wire string (`<= 3,600` frames). */
export function encodeDockingTape(frames: readonly DockingFrameInput[]): string {
  const capped = frames.slice(0, 3_600);
  const spans: Array<{ mask: number; count: number }> = [];
  for (const frame of capped) {
    const mask = encodeDockingInputMask(frame);
    const last = spans[spans.length - 1];
    if (last && last.mask === mask && last.count < 65_535) {
      last.count += 1;
    } else {
      spans.push({ mask, count: 1 });
    }
  }
  return spans.map((s) => `${s.mask.toString(16).padStart(4, "0")}*${s.count}`).join(",");
}

/** Convert a `dockOrder` command string into a single 60 Hz `DockingFrameInput`. */
export function dockOrderToFrameInput(cmd: string): DockingFrameInput | null {
  const base: DockingFrameInput = {
    transX: 0,
    transY: 0,
    closurePulse: 0,
    pitchTrim: 0,
    yawTrim: 0,
    rollTrim: 0,
  };
  switch (cmd) {
    case "DOCK TRANS UP":
      return { ...base, transY: 1 };
    case "DOCK TRANS DOWN":
      return { ...base, transY: -1 };
    case "DOCK TRANS LEFT":
      return { ...base, transX: -1 };
    case "DOCK TRANS RIGHT":
      return { ...base, transX: 1 };
    case "DOCK PITCH UP":
      return { ...base, pitchTrim: 1 };
    case "DOCK PITCH DOWN":
      return { ...base, pitchTrim: -1 };
    case "DOCK YAW LEFT":
      return { ...base, yawTrim: -1 };
    case "DOCK YAW RIGHT":
      return { ...base, yawTrim: 1 };
    case "DOCK ROLL CCW":
      return { ...base, rollTrim: -1 };
    case "DOCK ROLL CW":
      return { ...base, rollTrim: 1 };
    case "DOCK CLOSURE FORWARD":
      return { ...base, closurePulse: 1 };
    case "DOCK CLOSURE REVERSE":
      return { ...base, closurePulse: -1 };
    case "DOCK DAMPENERS TOGGLE":
      return { ...base, toggleDampeners: true };
    default:
      return null;
  }
}

/** Convert a `dockOrder` command string into its packed 16-bit RLE bitmask for `mush-pixel-wasm` (`mp_docking_step`). */
export function dockOrderToInputMask(cmd: string): number | null {
  const input = dockOrderToFrameInput(cmd);
  return input === null ? null : encodeDockingInputMask(input);
}
