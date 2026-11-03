// The cockpit's picture in the web client (doc 21 cut G.3c): the gateway
// draws the first-person view from this connection's own feeds and sends it
// as one PNG a binary message while the client has asked for it with the
// `picture` command (G.3a). Everything here is pure — the command, the look's
// swing, the helm's lines — so vitest pins it without a socket or a canvas.
//
// The look is the camera's offset from the tank's heading and is the
// viewer's alone: it rides the `picture` command and the world never hears of
// it. Turning the tank is a `HEADING` line like any typed order, which the
// picture's arrow keys send exactly as the SSH TUI's do (G.4): the one place
// the cockpit tab sends an order rather than prefilling one.
//
// Doc 21 cut G.12c binds the rest of §7.3's flight keys, the same letters the
// TUI's screen 0 reads: `a`/`d` turn as the arrows do, `w`/`s` step the
// ordered speed, `x` all stop, `q`/`e` look, `f` the weapons gate, `g` the
// volley, `t`/`T` the lock cycle, `v` the overhead, and `F1`–`F6` the
// `ORDER` prefills (the one family that still prefills rather than sends).
// The stepping, the clamp and the lock order are pure here so the TUI's
// rules and the web's are pinned side by side.

import type { CockpitContact } from "./cockpit";
import type { ClientCommand } from "./proto";

/** How fast a held look swings, millidegrees a second (the TUI's 45°/s). */
export const LOOK_MDEG_PER_S = 45_000;

/** How often a swinging look is sent again, milliseconds (10 Hz). */
export const LOOK_STEP_MS = 100;

/** How far ahead of the tank a held turn orders its heading, degrees. A
 * pulse and a half at the stock tank's turn rate; the order is extended while
 * the key is held. */
export const HELM_LEAD_DEG = 30;

/** How often a held turn's order is extended, milliseconds. */
export const HELM_REPEAT_MS = 700;

/** Default hull turn rate in millidegrees per second (30°/s, matching TUI `Helm`). */
export const DEFAULT_TURN_RATE_MDEG_S = 30_000;

/** Minimum heading nudge for a quick real-time tap of `a`/`d`, millidegrees (5°). */
export const MIN_TAP_TURN_MDEG = 5_000;

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

/**
 * The `picture` command: on or off, at a look, of a kind (doc 21 cut G.12c
 * added `kind`; G.12a taught the gateway and the generated `ClientCommand`
 * it, the overhead drawn by `mush_pixel::overhead`).
 */
export function pictureCommand(
  on: boolean,
  lookMdeg: number,
  kind: PictureKind = "cockpit",
): ClientCommand {
  const wireKind = kind === "overhead" ? "overhead" : "cockpit";
  return { t: "picture", d: { on, look_mdeg: wrapMdeg(lookMdeg), kind: wireKind } };
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

/**
 * The helm's order: `HEADING <deg>` for a turn `dir` from `headingDeg` —
 * [`HELM_LEAD_DEG`] ahead to the left (`-1`) or right (`1`), or the heading
 * itself (`0`), which stops a turn where the tank is pointing.
 */
export function helmLine(headingDeg: number, dir: -1 | 0 | 1): string {
  // `QUIET` (doc 21 §7.3, 2026-10-27): the arena takes a key's order without
  // the `Steering to …°.` a typed one is acknowledged with — a held turn
  // sends one every 700 ms and the header shows the ordered heading.
  const deg = (((Math.round(headingDeg + dir * HELM_LEAD_DEG) % 360) + 360) % 360);
  return `HEADING ${deg} QUIET`;
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
  /** All stop: `SPEED 0` and the helm's stop. */
  | { kind: "allStop" }
  /** First person ↔ overhead. */
  | { kind: "view" }
  /** Weapons free ↔ weapons hold (`FIRE FREE` / `FIRE HOLD`). */
  | { kind: "gate" }
  /** One volley (`FIRE ALL`). */
  | { kind: "volley" }
  /** Fire an individual weapon slot `0..=3` (`Shift+1`..`Shift+4` -> `!`, `@`, `#`, `$`). */
  | { kind: "fireSlot"; slot: 0 | 1 | 2 | 3 }
  /** The next (`1`) or previous (`-1`) contact locked (`LOCK`). */
  | { kind: "lock"; dir: -1 | 1 }
  /** Zoom in (`1`) or out (`-1`) on the active cockpit camera. */
  | { kind: "zoom"; dir: -1 | 1 }
  /** The chat line opened with an `ORDER` ready. */
  | { kind: "order"; slot: OrderSlot };

/**
 * The picture's keys (doc 21 §7.3, cut G.12c): the TUI's screen-0 letters.
 *
 * Letters read either case — a caps lock must not take the cockpit away —
 * except `t`, whose shifted `T` is the previous lock as on the TUI. The
 * caller drops a key chorded with Ctrl, Alt or Meta: those belong to the
 * browser.
 */
export function pictureKey(key: string): PictureKey | null {
  switch (key) {
    case "ArrowLeft":
    case "a":
    case "A":
      return { kind: "turn", dir: -1 };
    case "ArrowRight":
    case "d":
    case "D":
      return { kind: "turn", dir: 1 };
    case "w":
    case "W":
      return { kind: "speed", dir: 1 };
    case "s":
    case "S":
      return { kind: "speed", dir: -1 };
    case "x":
    case "X":
      return { kind: "allStop" };
    case ",":
    case "q":
    case "Q":
      return { kind: "look", dir: -1 };
    case ".":
    case "e":
    case "E":
      return { kind: "look", dir: 1 };
    case "c":
    case "C":
      return { kind: "centre" };
    case " ":
      return { kind: "steady" };
    case "v":
    case "V":
      return { kind: "view" };
    case "f":
    case "F":
      return { kind: "gate" };
    case "g":
    case "G":
      return { kind: "volley" };
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

/** How often a speed step is sent at most, milliseconds: presses inside the
 * window coalesce and the last value wins, so a held `w` climbs four steps a
 * second rather than flooding the helm with the key's repeat rate. */
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

/** The `SPEED` line. `QUIET`, as `helmLine`'s: a key's order is taken without
 * the `Making … kph.` a typed one is acknowledged with (doc 21 §7.3). */
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

/** The header's word for the gate. */
export function gateWord(weaponsFree: boolean | null): string {
  return weaponsFree === false ? "weapons hold" : "weapons free";
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
 * then the rest, nearest first within each (the frame's own order breaks a
 * tie). */
export function lockOrder(contacts: readonly CockpitContact[]): string[] {
  return contacts
    .map((c, i) => ({ c, i }))
    .sort((a, b) => lockRank(a.c) - lockRank(b.c) || a.c.range_m - b.c.range_m || a.i - b.i)
    .map(({ c }) => c.designator);
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

/** Step the camera zoom in permille (`1000..=2000` in first-person `cockpit`, `500..=2000`
 * in `overhead` / `iso25d`, `1000` = 1.0x), or reset on `0`. */
export function stepZoomPermille(
  current: number,
  dir: -1 | 0 | 1,
  kind: PictureKind = "iso25d",
): number {
  if (dir === 0) return ZOOM_DEFAULT_PERMILLE;
  const minZoom = kind === "cockpit" ? ZOOM_DEFAULT_PERMILLE : ZOOM_MIN_PERMILLE;
  const base =
    Number.isFinite(current) && current > 0
      ? Math.max(minZoom, Math.round(current))
      : ZOOM_DEFAULT_PERMILLE;
  return Math.min(ZOOM_MAX_PERMILLE, Math.max(minZoom, base + dir * ZOOM_STEP_PERMILLE));
}

/** Pick the nearest contact on the `cockpit`, `overhead`, or `iso25d` canvas at normalized
 * `[0, 1]` coordinates `(normX, normY)`, returning its `designator` when within the hit radius. */
export function pickCanvasContact(
  contacts: readonly CockpitContact[],
  kind: PictureKind,
  normX: number,
  normY: number,
  zoomPermille = ZOOM_DEFAULT_PERMILLE,
  sightM = 600,
  cameraHeadingDeg = 0,
): string | null {
  if (!Number.isFinite(normX) || !Number.isFinite(normY)) return null;
  const px = normX * 640;
  const py = normY * 480;
  if (kind === "cockpit") {
    const z =
      Math.min(
        ZOOM_MAX_PERMILLE,
        Math.max(ZOOM_DEFAULT_PERMILLE, zoomPermille > 0 ? zoomPermille : ZOOM_DEFAULT_PERMILLE),
      ) / 1_000;
    let bestDesig: string | null = null;
    let bestDistSq = 44 * 44;
    for (const c of contacts) {
      if (!c.designator) continue;
      const relDeg = ((((c.bearing_deg - cameraHeadingDeg + 180) % 360) + 360) % 360) - 180;
      if (Math.abs(relDeg) > 36) continue;
      const rad = (relDeg * Math.PI) / 180;
      const depth = c.range_m * Math.cos(rad);
      if (depth <= 1) continue;
      const planeX = (c.range_m * Math.sin(rad)) / (depth * 0.66);
      const cx = 320 + planeX * 320 * z;
      const cy = 240;
      if (cx < -32 || cx > 672) continue;
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
    const z =
      Math.min(
        ZOOM_MAX_PERMILLE,
        Math.max(ZOOM_MIN_PERMILLE, zoomPermille > 0 ? zoomPermille : ZOOM_DEFAULT_PERMILLE),
      ) / 1_000;
    const scale = 2.0 * z;
    pxPerMX = 1.15 * scale;
    pxPerMY = 0.78 * scale;
  } else {
    const z =
      Math.min(
        ZOOM_MAX_PERMILLE,
        Math.max(ZOOM_MIN_PERMILLE, zoomPermille > 0 ? zoomPermille : ZOOM_DEFAULT_PERMILLE),
      ) / 1_000;
    const baseReach = Math.min(3_000, Math.max(150, (sightM > 0 ? sightM : 600) * 1.15));
    const reach = Math.min(6_000, Math.max(75, baseReach / z));
    const pxPerM = 240 / reach;
    pxPerMX = pxPerM;
    pxPerMY = pxPerM;
  }

  let bestDesig: string | null = null;
  let bestDistSq = 34 * 34;
  for (const c of contacts) {
    if (!c.designator) continue;
    const rad = (c.bearing_deg * Math.PI) / 180;
    const cx = 320 + c.range_m * Math.sin(rad) * pxPerMX;
    const cy = 240 - c.range_m * Math.cos(rad) * pxPerMY;
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
 * the centre deadzone. */
export function canvasClickHeadingDeg(
  kind: PictureKind,
  normX: number,
  normY: number,
): number | null {
  if (kind !== "iso25d" && kind !== "overhead") return null;
  if (!Number.isFinite(normX) || !Number.isFinite(normY)) return null;
  const dxPx = normX * 640 - 320;
  const dyPx = 240 - normY * 480;
  if (dxPx * dxPx + dyPx * dyPx < 18 * 18) return null;
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
 */
export function spacePictureKey(key: string, mode: SpaceCameraMode): SpacePictureKey | null {
  if (key === "v" || key === "V") {
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
      case "c":
      case "C":
      case " ":
        return { kind: "dockOrder", cmd: "DOCK CLAMP ASSIST" };
      default:
        return null;
    }
  }
  if (mode === "holotank") {
    switch (key) {
      case ",":
      case "a":
      case "A":
      case "ArrowLeft":
        return { kind: "holotankOrbit", deltaMdeg: -15_000 };
      case ".":
      case "d":
      case "D":
      case "ArrowRight":
        return { kind: "holotankOrbit", deltaMdeg: 15_000 };
      case "z":
      case "Z":
      case "F1":
        return { kind: "spaceOrder", cmd: "+tac/beam" };
      case "x":
      case "X":
      case "F2":
        return { kind: "spaceOrder", cmd: "+tac/torp" };
      case "c":
      case "C":
      case "F3":
        return { kind: "spaceOrder", cmd: "+tac/railgun" };
      case "r":
      case "R":
        return { kind: "spaceOrder", cmd: "+tac/scan" };
      default:
        return null;
    }
  }
  // `space_cockpit`: camera-local look and `+tac/*` weapon group fire keys (`SF.R13`, matching TUI `pixel.rs`).
  switch (key) {
    case ",":
    case "q":
    case "Q":
      return { kind: "look", dir: -1 };
    case ".":
    case "e":
    case "E":
      return { kind: "look", dir: 1 };
    case "z":
    case "Z":
    case "F1":
      return { kind: "spaceOrder", cmd: "+tac/beam" };
    case "x":
    case "X":
    case "F2":
      return { kind: "spaceOrder", cmd: "+tac/torp" };
    case "c":
    case "C":
    case "F3":
      return { kind: "spaceOrder", cmd: "+tac/railgun" };
    case "r":
    case "R":
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

/** Per-hull 6-DOF RCS thruster authority parameters (`SF.R16`, `D-35-14`). */
export interface WebDockingRcsAuthority {
  transDvMmpsPerFrame: number;
  closureDvMmpsPerFrame: number;
  attitudeRateMdegSPerFrame: number;
  rollRateMdegSPerFrame: number;
  dampTransMmpsPerFrame: number;
  dampAttitudeMdegSPerFrame: number;
  dampRollMdegSPerFrame: number;
  kgPerPulseBatch: number;
}

/** Default 6-DOF RCS authority (`Dray-P` / standard freighter). */
export const DEFAULT_WEB_DOCKING_RCS_AUTHORITY: WebDockingRcsAuthority = {
  transDvMmpsPerFrame: 30,
  closureDvMmpsPerFrame: 25,
  attitudeRateMdegSPerFrame: 180,
  rollRateMdegSPerFrame: 240,
  dampTransMmpsPerFrame: 15,
  dampAttitudeMdegSPerFrame: 90,
  dampRollMdegSPerFrame: 120,
  kgPerPulseBatch: 1,
};

/** Client-side 60 Hz 6-DOF docking game session (`SF.R16`, `D-35-14`). */
export interface WebDockingSession {
  sessionId: string;
  dxMm: number;
  dyMm: number;
  dzMm: number;
  vxMmps: number;
  vyMmps: number;
  vzMmps: number;
  pitchMdeg: number;
  yawMdeg: number;
  rollMdeg: number;
  pitchRateMdegS: number;
  yawRateMdegS: number;
  rollRateMdegS: number;
  remX: number;
  remY: number;
  remZ: number;
  remPitch: number;
  remYaw: number;
  remRoll: number;
  dampenersOn: boolean;
  rcsAuthority: WebDockingRcsAuthority;
  targetTumbleMdegS: [number, number, number];
  maxFrames: number;
  framesElapsed: number;
  pulseFrames: number;
  recordedFrames: DockingFrameInput[];
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

/** Initialize a 60 Hz `WebDockingSession` from a `space.docking` keyframe (`SF.R16`). */
export function createWebDockingSession(feed: unknown): WebDockingSession | null {
  if (!feed || typeof feed !== "object") return null;
  const obj = feed as Record<string, unknown>;
  const sessionId = typeof obj.session_id === "string" && obj.session_id.length > 0 ? obj.session_id : null;
  if (!sessionId) return null;

  const num = (v: unknown, def: number): number => (typeof v === "number" && Number.isFinite(v) ? Math.trunc(v) : def);
  const rawAuth =
    obj.rcs_authority && typeof obj.rcs_authority === "object"
      ? (obj.rcs_authority as Record<string, unknown>)
      : null;
  const rcsAuthority: WebDockingRcsAuthority = rawAuth
    ? {
        transDvMmpsPerFrame: num(rawAuth.trans_dv_mmps_per_frame, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.transDvMmpsPerFrame),
        closureDvMmpsPerFrame: num(rawAuth.closure_dv_mmps_per_frame, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.closureDvMmpsPerFrame),
        attitudeRateMdegSPerFrame: num(
          rawAuth.attitude_rate_mdeg_s_per_frame,
          DEFAULT_WEB_DOCKING_RCS_AUTHORITY.attitudeRateMdegSPerFrame,
        ),
        rollRateMdegSPerFrame: num(rawAuth.roll_rate_mdeg_s_per_frame, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.rollRateMdegSPerFrame),
        dampTransMmpsPerFrame: num(rawAuth.damp_trans_mmps_per_frame, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.dampTransMmpsPerFrame),
        dampAttitudeMdegSPerFrame: num(
          rawAuth.damp_attitude_mdeg_s_per_frame,
          DEFAULT_WEB_DOCKING_RCS_AUTHORITY.dampAttitudeMdegSPerFrame,
        ),
        dampRollMdegSPerFrame: num(rawAuth.damp_roll_mdeg_s_per_frame, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.dampRollMdegSPerFrame),
        kgPerPulseBatch: num(rawAuth.kg_per_pulse_batch, DEFAULT_WEB_DOCKING_RCS_AUTHORITY.kgPerPulseBatch),
      }
    : { ...DEFAULT_WEB_DOCKING_RCS_AUTHORITY };

  const tumbleArr = Array.isArray(obj.target_tumble_mdeg_s) ? obj.target_tumble_mdeg_s : [0, 0, 0];
  const targetTumbleMdegS: [number, number, number] = [
    num(tumbleArr[0], 0),
    num(tumbleArr[1], 0),
    num(tumbleArr[2], 0),
  ];

  return {
    sessionId,
    dxMm: num(obj.dx_mm, 0),
    dyMm: num(obj.dy_mm, 0),
    dzMm: num(obj.dz_mm, 2400),
    vxMmps: num(obj.vx_mmps, 0),
    vyMmps: num(obj.vy_mmps, 0),
    vzMmps: num(obj.vz_mmps, 0),
    pitchMdeg: num(obj.pitch_mdeg, 0),
    yawMdeg: num(obj.yaw_mdeg, 0),
    rollMdeg: num(obj.roll_mdeg, 0),
    pitchRateMdegS: num(obj.pitch_rate_mdeg_s, 0),
    yawRateMdegS: num(obj.yaw_rate_mdeg_s, 0),
    rollRateMdegS: num(obj.roll_rate_mdeg_s, 0),
    remX: 0,
    remY: 0,
    remZ: 0,
    remPitch: 0,
    remYaw: 0,
    remRoll: 0,
    dampenersOn: typeof obj.dampeners === "boolean" ? obj.dampeners : true,
    rcsAuthority,
    targetTumbleMdegS,
    maxFrames: Math.min(num(obj.max_frames, 600), 3600),
    framesElapsed: 0,
    pulseFrames: 0,
    recordedFrames: [],
  };
}

/** Count how many of the 5 collar alignment gates (`D-35-14`) are green. */
export function countWebDockingGatesGreen(s: WebDockingSession): number {
  let count = 0;
  if (Math.abs(s.dxMm) <= 250 && Math.abs(s.dyMm) <= 250 && Math.abs(s.vxMmps) <= 300 && Math.abs(s.vyMmps) <= 300) {
    count += 1;
  }
  if (
    Math.abs(s.pitchMdeg) <= 2000 &&
    Math.abs(s.yawMdeg) <= 2000 &&
    Math.abs(s.pitchRateMdegS) <= 500 &&
    Math.abs(s.yawRateMdegS) <= 500
  ) {
    count += 1;
  }
  if (Math.abs(s.rollMdeg) <= 3000 && Math.abs(s.rollRateMdegS) <= 500) {
    count += 1;
  }
  if (s.vzMmps >= 100 && s.vzMmps <= 800) {
    count += 1;
  }
  if (s.dzMm >= 0 && s.dzMm <= 500) {
    count += 1;
  }
  return count;
}

/** True when all 5 collar gates are green (`dz <= 500 mm`, `100 <= vz <= 800 mm/s`, aligned). */
export function isWebDockingClamped(s: WebDockingSession): boolean {
  return countWebDockingGatesGreen(s) === 5;
}

/** Step one 60 Hz frame (`1/60 s`) of the client-side 6-DOF docking kinematics (`SF.R16`, `D-35-14`). */
export function stepWebDockingFrame(s: WebDockingSession, input: DockingFrameInput): void {
  if (s.recordedFrames.length >= s.maxFrames) return;
  s.recordedFrames.push(input);
  if (input.abort) return;
  if (input.toggleDampeners) {
    s.dampenersOn = !s.dampenersOn;
  }
  if (
    input.transX !== 0 ||
    input.transY !== 0 ||
    input.closurePulse !== 0 ||
    input.pitchTrim !== 0 ||
    input.yawTrim !== 0 ||
    input.rollTrim !== 0
  ) {
    s.pulseFrames += 1;
  }

  const auth = s.rcsAuthority;
  s.vxMmps += input.transX * auth.transDvMmpsPerFrame;
  s.vyMmps += input.transY * auth.transDvMmpsPerFrame;
  s.vzMmps += input.closurePulse * auth.closureDvMmpsPerFrame;
  s.pitchRateMdegS += input.pitchTrim * auth.attitudeRateMdegSPerFrame;
  s.yawRateMdegS += input.yawTrim * auth.attitudeRateMdegSPerFrame;
  s.rollRateMdegS += input.rollTrim * auth.rollRateMdegSPerFrame;

  const dampStep = (val: number, step: number): number => {
    if (val > 0) return Math.max(0, val - step);
    if (val < 0) return Math.min(0, val + step);
    return 0;
  };
  if (s.dampenersOn) {
    if (input.transX === 0) s.vxMmps = dampStep(s.vxMmps, auth.dampTransMmpsPerFrame);
    if (input.transY === 0) s.vyMmps = dampStep(s.vyMmps, auth.dampTransMmpsPerFrame);
    if (input.pitchTrim === 0) s.pitchRateMdegS = dampStep(s.pitchRateMdegS, auth.dampAttitudeMdegSPerFrame);
    if (input.yawTrim === 0) s.yawRateMdegS = dampStep(s.yawRateMdegS, auth.dampAttitudeMdegSPerFrame);
    if (input.rollTrim === 0) s.rollRateMdegS = dampStep(s.rollRateMdegS, auth.dampRollMdegSPerFrame);
  }

  const stepAxis60 = (pos: number, rem: number, rate: number): [number, number] => {
    const total = rem + rate;
    const delta = Math.trunc(total / 60);
    const nextRem = total - delta * 60;
    return [pos + delta, nextRem];
  };

  [s.dxMm, s.remX] = stepAxis60(s.dxMm, s.remX, s.vxMmps);
  [s.dyMm, s.remY] = stepAxis60(s.dyMm, s.remY, s.vyMmps);
  const totZ = s.vzMmps + s.remZ;
  const deltaZ = Math.trunc(totZ / 60);
  s.dzMm = Math.max(0, s.dzMm - deltaZ);
  s.remZ = totZ - deltaZ * 60;

  [s.pitchMdeg, s.remPitch] = stepAxis60(s.pitchMdeg, s.remPitch, s.pitchRateMdegS - s.targetTumbleMdegS[0]);
  [s.yawMdeg, s.remYaw] = stepAxis60(s.yawMdeg, s.remYaw, s.yawRateMdegS - s.targetTumbleMdegS[1]);
  [s.rollMdeg, s.remRoll] = stepAxis60(s.rollMdeg, s.remRoll, s.rollRateMdegS - s.targetTumbleMdegS[2]);
  s.framesElapsed += 1;
}

/** Compute the next 60 Hz guidance-assist `DockingFrameInput` toward all 5 collar gates (`SF.R16`). */
export function nextWebDockingGuidanceInput(s: WebDockingSession): DockingFrameInput {
  const auth = s.rcsAuthority;
  const tumble = s.targetTumbleMdegS;
  const input: DockingFrameInput = {
    transX: 0,
    transY: 0,
    closurePulse: 0,
    pitchTrim: 0,
    yawTrim: 0,
    rollTrim: 0,
  };

  const dvTrans = Math.max(auth.transDvMmpsPerFrame, 5);
  const dvClose = Math.max(auth.closureDvMmpsPerFrame, 5);
  const dAtt = Math.max(auth.attitudeRateMdegSPerFrame, 30);
  const dRoll = Math.max(auth.rollRateMdegSPerFrame, 30);

  const fastVx = Math.min(dvTrans * 6, 240);
  const slowVx = Math.min(dvTrans * 2, 120);
  const deadVx = Math.max(Math.trunc(dvTrans / 2), 5);
  const targetVx =
    s.dxMm > 120 ? -fastVx : s.dxMm < -120 ? fastVx : s.dxMm > 20 ? -slowVx : s.dxMm < -20 ? slowVx : 0;
  if (s.vxMmps < targetVx - deadVx) {
    input.transX = 1;
  } else if (s.vxMmps > targetVx + deadVx) {
    input.transX = -1;
  }

  const targetVy =
    s.dyMm > 120 ? -fastVyOrVx(fastVx) : s.dyMm < -120 ? fastVx : s.dyMm > 20 ? -slowVx : s.dyMm < -20 ? slowVx : 0;
  if (s.vyMmps < targetVy - deadVx) {
    input.transY = 1;
  } else if (s.vyMmps > targetVy + deadVx) {
    input.transY = -1;
  }

  const fastAtt = Math.min(dAtt * 2, 420);
  const slowAtt = Math.min(dAtt, 240);
  const deadAtt = Math.max(Math.trunc(dAtt / 2), 15);
  const targetPitchRate =
    s.pitchMdeg > 600
      ? -fastAtt
      : s.pitchMdeg < -600
        ? fastAtt
        : s.pitchMdeg > 90
          ? -slowAtt
          : s.pitchMdeg < -90
            ? slowAtt
            : tumble[0];
  if (s.pitchRateMdegS < targetPitchRate - deadAtt) {
    input.pitchTrim = 1;
  } else if (s.pitchRateMdegS > targetPitchRate + deadAtt) {
    input.pitchTrim = -1;
  }

  const targetYawRate =
    s.yawMdeg > 600
      ? -fastAtt
      : s.yawMdeg < -600
        ? fastAtt
        : s.yawMdeg > 90
          ? -slowAtt
          : s.yawMdeg < -90
            ? slowAtt
            : tumble[1];
  if (s.yawRateMdegS < targetYawRate - deadAtt) {
    input.yawTrim = 1;
  } else if (s.yawRateMdegS > targetYawRate + deadAtt) {
    input.yawTrim = -1;
  }

  const fastRoll = Math.min(dRoll * 2, 480);
  const slowRoll = Math.min(dRoll, 240);
  const deadRoll = Math.max(Math.trunc(dRoll / 2), 15);
  const targetRollRate =
    s.rollMdeg > 900
      ? -fastRoll
      : s.rollMdeg < -900
        ? fastRoll
        : s.rollMdeg > 120
          ? -slowRoll
          : s.rollMdeg < -120
            ? slowRoll
            : tumble[2];
  if (s.rollRateMdegS < targetRollRate - deadRoll) {
    input.rollTrim = 1;
  } else if (s.rollRateMdegS > targetRollRate + deadRoll) {
    input.rollTrim = -1;
  }

  const latOk =
    Math.abs(s.dxMm) <= 250 &&
    Math.abs(s.dyMm) <= 250 &&
    Math.abs(s.vxMmps) <= 300 &&
    Math.abs(s.vyMmps) <= 300;
  const attOk =
    Math.abs(s.pitchMdeg) <= 2000 &&
    Math.abs(s.yawMdeg) <= 2000 &&
    Math.abs(s.pitchRateMdegS) <= 500 &&
    Math.abs(s.yawRateMdegS) <= 500;
  const rollOk = Math.abs(s.rollMdeg) <= 3000 && Math.abs(s.rollRateMdegS) <= 500;
  const targetClosure = latOk && attOk && rollOk ? 350 : 0;
  const deadClose = Math.max(Math.min(dvClose, 20), 5);
  if (s.vzMmps < targetClosure - deadClose) {
    input.closurePulse = 1;
  } else if (s.vzMmps > targetClosure + deadClose) {
    input.closurePulse = -1;
  }

  return input;
}

function fastVyOrVx(v: number): number {
  return v;
}

/** Play the 60 Hz client docking game to collar clamp (`SF.R16`). */
export function playWebDockingToClamp(s: WebDockingSession): boolean {
  while (s.recordedFrames.length < s.maxFrames) {
    if (isWebDockingClamped(s) && s.recordedFrames.length > 0) return true;
    if (s.dzMm === 0 && !isWebDockingClamped(s)) return false;
    const next = nextWebDockingGuidanceInput(s);
    stepWebDockingFrame(s, next);
    if (isWebDockingClamped(s)) return true;
  }
  return isWebDockingClamped(s);
}

/** Format the `+helm/dock tape <session_id> <rle>` command for server verification (`SF.R16`). */
export function formatWebDockingTapeCommand(s: WebDockingSession): string {
  return `+helm/dock tape ${s.sessionId} ${encodeDockingTape(s.recordedFrames)}`;
}

/** Overlay the live 60 Hz `WebDockingSession` pose onto a `space.docking` feed object. */
export function webDockingFeedOverlay(s: WebDockingSession, baseFeed: unknown): Record<string, unknown> {
  const base = baseFeed && typeof baseFeed === "object" ? (baseFeed as Record<string, unknown>) : {};
  const greenCount = countWebDockingGatesGreen(s);
  return {
    ...base,
    session_id: s.sessionId,
    dx_mm: s.dxMm,
    dy_mm: s.dyMm,
    dz_mm: s.dzMm,
    vx_mmps: s.vxMmps,
    vy_mmps: s.vyMmps,
    vz_mmps: s.vzMmps,
    pitch_mdeg: s.pitchMdeg,
    yaw_mdeg: s.yawMdeg,
    roll_mdeg: s.rollMdeg,
    pitch_rate_mdeg_s: s.pitchRateMdegS,
    yaw_rate_mdeg_s: s.yawRateMdegS,
    roll_rate_mdeg_s: s.rollRateMdegS,
    dampeners: s.dampenersOn,
    gates_green: greenCount,
    status: greenCount === 5 ? "ALIGNED-READY" : "APPROACHING",
  };
}
