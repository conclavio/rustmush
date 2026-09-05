// The key model of the modal web shell (interface plan I4, D-I-17, D-I-18,
// D-I-19) — the same table the TUI honours (§7 of the plan), decided as a
// pure function so vitest pins it without a DOM.
//
// Input mode is "a text field has focus". While nothing has focus the bare
// keys act; while a field has focus only Alt+key (the key's nav action) and
// Ctrl/⌘+K (the palette) are the shell's, everything else is the field's.

import type { RoomView } from "./proto";

/** Keys every screen inherits and may not rebind (plan §7). */
export const GLOBAL_KEYS: ReadonlySet<string> = new Set([
  "1",
  "2",
  "3",
  "4",
  "5",
  "6",
  "7",
  "8",
  "9",
  '"',
  ":",
  ";",
  "\\",
  "/",
  "?",
]);

/** `g` / `G` jump the focused pane to its top and bottom. */
export const JUMP_KEYS: ReadonlySet<string> = new Set(["g", "G"]);

/** The letters the scene screen never binds itself, so a room's exits can
 * always use them (D-I-19). */
export const RESERVED_EXIT_KEYS: readonly string[] = ["n", "e", "s", "w", "u", "d", "o"];

/** The speech triggers that open the composer with themselves typed. */
export const TRIGGER_KEYS: ReadonlySet<string> = new Set(['"', ":", ";", "\\"]);

/** A keyboard event reduced to what the model needs. */
export interface KeyInput {
  /** `KeyboardEvent.key`. */
  key: string;
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  /** Whether a text field (input, textarea, contenteditable) has focus. */
  inField: boolean;
}

/** What a key asks the shell to do. */
export type NavIntent =
  | { kind: "screen"; digit: number }
  | { kind: "compose"; prefill: string }
  | { kind: "palette" }
  | { kind: "help" }
  | { kind: "pane"; dir: 1 | -1 }
  | { kind: "scroll"; rows: number }
  | { kind: "jump"; bottom: boolean }
  | { kind: "exit"; name: string }
  | { kind: "binding"; key: string };

/** Rows a page key moves. */
export const PAGE_ROWS = 10;

/**
 * The shell's intent for one key, or `null` when the key is not the
 * shell's (a field's own editing key, a browser shortcut, an unbound key).
 *
 * `exits` maps a one-letter key to the exit name it walks (scene screen
 * only — pass an empty map elsewhere); `bindings` is the active screen's
 * own keys.
 */
export function navIntent(
  input: KeyInput,
  exits: ReadonlyMap<string, string>,
  bindings: ReadonlySet<string>,
): NavIntent | null {
  const { key } = input;
  const accel = input.ctrl || input.meta;
  if (accel && !input.alt && key.toLowerCase() === "k") {
    return { kind: "palette" };
  }
  if (input.inField) {
    // Alt+key is the key's nav action from inside the box (D-I-17).
    if (input.alt && !accel) {
      return bareIntent(key, exits, bindings, true);
    }
    return null;
  }
  if (accel || input.alt) {
    // Browser shortcuts (Ctrl+L, ⌘+R …) stay the browser's; Alt+key with
    // nothing focused is the same as the bare key.
    return input.alt && !accel ? bareIntent(key, exits, bindings, true) : null;
  }
  return bareIntent(key, exits, bindings, false);
}

/** The bare-key table; `fromBox` limits it to the actions Alt+key reaches. */
function bareIntent(
  key: string,
  exits: ReadonlyMap<string, string>,
  bindings: ReadonlySet<string>,
  fromBox: boolean,
): NavIntent | null {
  if (/^[1-9]$/.test(key)) {
    return { kind: "screen", digit: Number(key) };
  }
  if (key === "?") {
    return { kind: "help" };
  }
  if (!fromBox) {
    if (key === "Enter") {
      return { kind: "compose", prefill: "" };
    }
    if (TRIGGER_KEYS.has(key)) {
      return { kind: "compose", prefill: key };
    }
    if (key === "/") {
      return { kind: "palette" };
    }
    if (key === "Tab") {
      return { kind: "pane", dir: 1 };
    }
    if (key === "ArrowUp") {
      return { kind: "scroll", rows: -1 };
    }
    if (key === "ArrowDown") {
      return { kind: "scroll", rows: 1 };
    }
    if (key === "PageUp") {
      return { kind: "scroll", rows: -PAGE_ROWS };
    }
    if (key === "PageDown") {
      return { kind: "scroll", rows: PAGE_ROWS };
    }
    if (key === "g") {
      return { kind: "jump", bottom: false };
    }
    if (key === "G") {
      return { kind: "jump", bottom: true };
    }
  }
  const exit = exits.get(key);
  if (exit !== undefined) {
    return { kind: "exit", name: exit };
  }
  if (bindings.has(key)) {
    return { kind: "binding", key };
  }
  return null;
}

/**
 * The one-letter exit keys a room offers (D-I-19): every exit whose name or
 * an alias is exactly one ASCII letter binds that letter (lower-cased),
 * unless the letter is a global or jump key; the first exit wins a
 * contested letter.
 */
export function exitKeys(view: RoomView | null): Map<string, string> {
  const keys = new Map<string, string>();
  if (!view) {
    return keys;
  }
  for (const exit of view.exits) {
    for (const cand of [exit.name, ...exit.aliases]) {
      const t = cand.trim();
      if (t.length !== 1 || !/^[a-zA-Z]$/.test(t)) {
        continue;
      }
      const c = t.toLowerCase();
      if (GLOBAL_KEYS.has(c) || JUMP_KEYS.has(c) || keys.has(c)) {
        continue;
      }
      keys.set(c, exit.name);
      break;
    }
  }
  return keys;
}

/** Whether a DOM event target is a text field (input mode). */
export function isTextField(target: unknown): boolean {
  if (typeof target !== "object" || target === null) {
    return false;
  }
  const el = target as { tagName?: string; isContentEditable?: boolean; type?: string };
  const tag = (el.tagName ?? "").toLowerCase();
  if (tag === "textarea" || tag === "select") {
    return true;
  }
  if (tag === "input") {
    const type = (el.type ?? "text").toLowerCase();
    return !["button", "checkbox", "radio", "submit", "range"].includes(type);
  }
  return el.isContentEditable === true;
}
