// The modal key model (interface plan I4): bare keys act while nothing has
// focus, Alt+key acts from inside the box, a bare key in the box is the
// box's, exits are keys on the scene screen, browser shortcuts stay the
// browser's.

import { describe, expect, it } from "vitest";

import { exitKeys, isTextField, navIntent, type KeyInput } from "./keys";
import type { RoomView } from "./proto";

function k(key: string, over: Partial<KeyInput> = {}): KeyInput {
  return { key, alt: false, ctrl: false, meta: false, shift: false, inField: false, ...over };
}

const exits = new Map([
  ["n", "North"],
  ["o", "Out"],
]);
const bindings = new Set(["l", "r"]);

describe("navIntent", () => {
  it("switches screens by bare digit when unfocused and by Alt+digit when focused", () => {
    expect(navIntent(k("3"), exits, bindings)).toEqual({ kind: "screen", digit: 3 });
    expect(navIntent(k("3", { inField: true }), exits, bindings)).toBeNull();
    expect(navIntent(k("3", { inField: true, alt: true }), exits, bindings)).toEqual({
      kind: "screen",
      digit: 3,
    });
    // 16b cut Q.5: `0` is the cockpit's digit, the one the TUI gave it. It is
    // still INERT on a server that publishes no cockpit — no available screen
    // has it, so `ScreensStore.switchDigit` refuses and `GameApp.act` answers
    // `false` — but the key model always answers the intent.
    expect(navIntent(k("0"), exits, bindings)).toEqual({ kind: "screen", digit: 0 });
    expect(navIntent(k("0", { inField: true }), exits, bindings)).toBeNull();
    expect(navIntent(k("0", { inField: true, alt: true }), exits, bindings)).toEqual({
      kind: "screen",
      digit: 0,
    });
  });

  it("a bare key while the composer is focused is the composer's", () => {
    for (const key of ["l", "n", "Enter", '"', "/", "?", "Tab", "ArrowUp"]) {
      expect(navIntent(k(key, { inField: true }), exits, bindings)).toBeNull();
    }
  });

  it("Enter and the triggers open the composer, / and ^K the palette, ? help", () => {
    expect(navIntent(k("Enter"), exits, bindings)).toEqual({ kind: "compose", prefill: "" });
    expect(navIntent(k('"'), exits, bindings)).toEqual({ kind: "compose", prefill: '"' });
    expect(navIntent(k(":"), exits, bindings)).toEqual({ kind: "compose", prefill: ":" });
    expect(navIntent(k("/"), exits, bindings)).toEqual({ kind: "palette" });
    expect(navIntent(k("k", { ctrl: true }), exits, bindings)).toEqual({ kind: "palette" });
    expect(navIntent(k("k", { meta: true, inField: true }), exits, bindings)).toEqual({
      kind: "palette",
    });
    expect(navIntent(k("?"), exits, bindings)).toEqual({ kind: "help" });
    expect(navIntent(k("?", { inField: true, alt: true }), exits, bindings)).toEqual({
      kind: "help",
    });
  });

  it("exits walk on the scene screen and Alt+letter walks from the box", () => {
    expect(navIntent(k("n"), exits, bindings)).toEqual({ kind: "exit", name: "North" });
    expect(navIntent(k("n", { inField: true, alt: true }), exits, bindings)).toEqual({
      kind: "exit",
      name: "North",
    });
    expect(navIntent(k("n"), new Map(), bindings)).toBeNull();
    expect(navIntent(k("l"), exits, bindings)).toEqual({ kind: "binding", key: "l" });
    expect(navIntent(k("l", { inField: true, alt: true }), exits, bindings)).toEqual({
      kind: "binding",
      key: "l",
    });
    expect(navIntent(k("z"), exits, bindings)).toBeNull();
  });

  it("pane, scroll and jump keys act only when unfocused", () => {
    expect(navIntent(k("Tab"), exits, bindings)).toEqual({ kind: "pane", dir: 1 });
    expect(navIntent(k("ArrowUp"), exits, bindings)).toEqual({ kind: "scroll", rows: -1 });
    expect(navIntent(k("PageDown"), exits, bindings)).toEqual({ kind: "scroll", rows: 10 });
    expect(navIntent(k("G"), exits, bindings)).toEqual({ kind: "jump", bottom: true });
    expect(navIntent(k("g", { inField: true, alt: true }), exits, bindings)).toBeNull();
  });

  it("browser shortcuts stay the browser's", () => {
    expect(navIntent(k("l", { ctrl: true }), exits, bindings)).toBeNull();
    expect(navIntent(k("r", { meta: true }), exits, bindings)).toBeNull();
    expect(navIntent(k("1", { ctrl: true }), exits, bindings)).toBeNull();
  });
});

describe("exitKeys", () => {
  const view: RoomView = {
    name: "Hall",
    id: "#2",
    desc: [],
    contents: [],
    exits: [
      { name: "North", aliases: ["n", "north"] },
      { name: "quai", aliases: [] },
      { name: "?", aliases: ["3"] },
      { name: "g", aliases: [] },
      { name: "Out", aliases: ["o"] },
      { name: "Other", aliases: ["o"] },
    ],
  };

  it("binds one-letter names and aliases, skipping globals and contested letters", () => {
    const keys = exitKeys(view);
    expect([...keys.entries()]).toEqual([
      ["n", "North"],
      ["o", "Out"],
    ]);
    expect(exitKeys(null).size).toBe(0);
  });
});

describe("isTextField", () => {
  it("recognises inputs, textareas and contenteditable, not buttons", () => {
    expect(isTextField({ tagName: "INPUT", type: "text" })).toBe(true);
    expect(isTextField({ tagName: "TEXTAREA" })).toBe(true);
    expect(isTextField({ tagName: "DIV", isContentEditable: true })).toBe(true);
    expect(isTextField({ tagName: "INPUT", type: "checkbox" })).toBe(false);
    expect(isTextField({ tagName: "BUTTON" })).toBe(false);
    expect(isTextField(null)).toBe(false);
  });
});
