// Theme — dark by default, light on request, "system" follows
// prefers-color-scheme. The explicit choice is stamped as
// `data-theme="dark" | "light"` on <html> (app.css keys off it); "system"
// removes the attribute so the media query decides. Persisted in
// localStorage (try/catch via $lib/storage).

import { storageGet, storageSet } from "./storage";

export type ThemeChoice = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";

const KEY = "rustmush.theme";

export function isThemeChoice(v: unknown): v is ThemeChoice {
  return v === "dark" || v === "light" || v === "system";
}

/** What a choice renders as, given the system preference. */
export function resolveTheme(choice: ThemeChoice, systemPrefersLight: boolean): ResolvedTheme {
  if (choice === "system") {
    return systemPrefersLight ? "light" : "dark";
  }
  return choice;
}

/** The next theme when the header button is clicked (flips the resolved look). */
export function toggledTheme(current: ThemeChoice, systemPrefersLight: boolean): ThemeChoice {
  return resolveTheme(current, systemPrefersLight) === "dark" ? "light" : "dark";
}

export function loadThemeChoice(): ThemeChoice {
  const stored = storageGet(KEY);
  return isThemeChoice(stored) ? stored : "dark";
}

/** Stamp the document and persist. Safe to call in non-DOM environments. */
export function applyTheme(choice: ThemeChoice): void {
  storageSet(KEY, choice);
  const root = globalThis.document?.documentElement;
  if (!root) {
    return;
  }
  if (choice === "system") {
    delete root.dataset.theme;
  } else {
    root.dataset.theme = choice;
  }
}

export function systemPrefersLight(): boolean {
  try {
    return globalThis.matchMedia?.("(prefers-color-scheme: light)").matches ?? false;
  } catch {
    return false;
  }
}
