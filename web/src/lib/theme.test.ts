// Theme resolution: choice × system preference, and the toggle direction.

import { describe, expect, it } from "vitest";

import { isThemeChoice, resolveTheme, toggledTheme } from "./theme";

describe("theme", () => {
  it("resolves system by preference and explicit choices as-is", () => {
    expect(resolveTheme("system", false)).toBe("dark");
    expect(resolveTheme("system", true)).toBe("light");
    expect(resolveTheme("dark", true)).toBe("dark");
    expect(resolveTheme("light", false)).toBe("light");
  });

  it("toggle flips the resolved look", () => {
    expect(toggledTheme("system", false)).toBe("light"); // looked dark → light
    expect(toggledTheme("system", true)).toBe("dark"); // looked light → dark
    expect(toggledTheme("dark", true)).toBe("light");
    expect(toggledTheme("light", true)).toBe("dark");
  });

  it("validates stored choices", () => {
    expect(isThemeChoice("dark")).toBe(true);
    expect(isThemeChoice("system")).toBe(true);
    expect(isThemeChoice("blue")).toBe(false);
    expect(isThemeChoice(null)).toBe(false);
  });
});
