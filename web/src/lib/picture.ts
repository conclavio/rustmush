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

/** Whether this viewer last wanted the picture (a per-viewer convenience). */
export const PICTURE_PREF_KEY = "rustmush.picture";

/** Which camera the picture is (doc 21 §7.4): the first-person view, or the
 * overhead — the arena north-up with the pilot's tank at its centre. */
export type PictureKind = "cockpit" | "overhead";

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
  return { t: "picture", d: { on, look_mdeg: wrapMdeg(lookMdeg), kind } };
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
  /** The next (`1`) or previous (`-1`) contact locked (`LOCK`). */
  | { kind: "lock"; dir: -1 | 1 }
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
    case "t":
      return { kind: "lock", dir: 1 };
    case "T":
      return { kind: "lock", dir: -1 };
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

/** A contact's band in the lock cycle: hostiles, then unidentified, then the
 * rest (the friends). The classes are `contactKind`'s. */
function lockRank(c: CockpitContact): number {
  if (c.class === "unknown") return 1;
  return c.friendly ? 2 : 0;
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
