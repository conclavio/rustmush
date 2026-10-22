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

/** The `picture` command: on or off, at a look. */
export function pictureCommand(on: boolean, lookMdeg: number): ClientCommand {
  return { t: "picture", d: { on, look_mdeg: wrapMdeg(lookMdeg) } };
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
  const deg = (((Math.round(headingDeg + dir * HELM_LEAD_DEG) % 360) + 360) % 360);
  return `HEADING ${deg}`;
}

/** What one key does to the picture, or `null` for a key it leaves alone. */
export type PictureKey =
  | { kind: "turn"; dir: -1 | 1 }
  | { kind: "look"; dir: -1 | 1 }
  | { kind: "centre" }
  | { kind: "stop" };

/** The picture's keys: the arrows turn the tank (held), `,` and `.` swing the
 * look (again to stop), `c` centres it, space stops both. */
export function pictureKey(key: string): PictureKey | null {
  switch (key) {
    case "ArrowLeft":
      return { kind: "turn", dir: -1 };
    case "ArrowRight":
      return { kind: "turn", dir: 1 };
    case ",":
      return { kind: "look", dir: -1 };
    case ".":
      return { kind: "look", dir: 1 };
    case "c":
    case "C":
      return { kind: "centre" };
    case " ":
      return { kind: "stop" };
    default:
      return null;
  }
}
