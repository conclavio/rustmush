// The composing signal (interface plan I4, D-I-11): one start per draft, a
// trailing stop on send, on a cleared box, or after idle.

import { describe, expect, it } from "vitest";

import { COMPOSING_IDLE_MS, ComposingSignal } from "./composing";

describe("ComposingSignal", () => {
  it("sends one start per draft and a trailing stop on send", () => {
    const s = new ComposingSignal();
    expect(s.keystroke(1000, "h")).toBe("start");
    expect(s.keystroke(1100, "he")).toBeNull();
    expect(s.keystroke(1200, "hel")).toBeNull();
    expect(s.active).toBe(true);
    expect(s.send()).toBe("stop");
    expect(s.send()).toBeNull();
    expect(s.active).toBe(false);
  });

  it("a cleared box stops, and typing again starts again", () => {
    const s = new ComposingSignal();
    expect(s.keystroke(0, "x")).toBe("start");
    expect(s.keystroke(10, "")).toBe("stop");
    expect(s.keystroke(20, "")).toBeNull();
    expect(s.keystroke(30, "y")).toBe("start");
  });

  it("idle silence stops the draft once", () => {
    const s = new ComposingSignal();
    s.keystroke(0, "x");
    expect(s.idle(COMPOSING_IDLE_MS - 1)).toBeNull();
    expect(s.idle(COMPOSING_IDLE_MS)).toBe("stop");
    expect(s.idle(COMPOSING_IDLE_MS + 1)).toBeNull();
    s.keystroke(COMPOSING_IDLE_MS + 2, "xy");
    expect(s.active).toBe(true);
    expect(s.idle(COMPOSING_IDLE_MS + 100)).toBeNull();
  });
});
