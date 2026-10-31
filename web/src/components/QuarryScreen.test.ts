// @vitest-environment jsdom
//
// QuarryScreen mounted (doc 20 cut SM.9; the lead's pre-commit round, the judge's
// correctness/F10): the Right pane's two sentences are the frame's own keys — `site.no_readings`
// and `site.other_rigs` — read whole, never picked out of `site.lines` by their opening words.
// The rig is a `QuarryStore` holding the blessed `site_map_other.json` frame, a `focusComposer`
// spy and a `refreshQuarry` spy — the whole of what the screen reaches for.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import QuarryScreen from "./QuarryScreen.svelte";
import type { GameApp } from "$lib/game.svelte";
import { QuarryStore } from "$lib/stores/quarry.svelte";

function fixture(name: string): { site: Record<string, unknown> } {
  return JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/site", name), "utf8"),
  ) as { site: Record<string, unknown> };
}

function mount(raw: unknown): void {
  const quarry = new QuarryStore();
  quarry.applySite(raw, Date.now());
  const app = { quarry, focusComposer: vi.fn(), refreshQuarry: vi.fn() } as unknown as GameApp;
  render(QuarryScreen, { props: { app } });
}

/** The side pane's paragraphs' text. */
function sideTexts(): string[] {
  const side = document.querySelector(".side");
  return [...(side?.querySelectorAll("p") ?? [])].map((p) => p.textContent ?? "");
}

describe("QuarryScreen's side pane", () => {
  it("shows the blessed frame's two sentences from their keys", () => {
    const raw = fixture("site_map_other.json");
    mount(raw);
    // Doc 32 cut CF.4: the second principal's rig in the bay sees the viewer's as a band.
    expect(sideTexts()).toEqual([raw.site.no_readings, raw.site.presence_line]);
    expect(raw.site.other_rigs).toBeNull();
  });

  it("shows a wrapped other-rigs sentence whole, and never reads site.lines", () => {
    const raw = fixture("site_map_other.json");
    const other =
      "Other rigs: 300 m NE (a sounding); 460 m N; 520 m NNE; 610 m E; 700 m SE; 820 m S; " +
      "940 m SW; 1060 m W; 1180 m NW; 1300 m WNW";
    raw.site.no_readings = "Nothing of metals read in view.";
    raw.site.other_rigs = other;
    // `site.lines` as line mode wraps it, and worded otherwise: the screen never reads it.
    raw.site.lines = [
      "You have no readings of metals in view.",
      "Rig 1: in the bay, charge 100 %, hopper empty.",
      "Other rigs: 300 m NE (a sounding); 460 m N; 520 m NNE; 610 m E; 700 m SE; 820 m S;",
      "  940 m SW; 1060 m W; 1180 m NW; 1300 m WNW",
      "Hold: 86 t of room unreserved.",
    ];
    mount(raw);
    expect(sideTexts()).toEqual(["Nothing of metals read in view.", other, raw.site.presence_line]);
    expect(screen.getByText(other).tagName).toBe("P");
  });

  it("shows neither sentence when the frame carries neither key", () => {
    const raw = fixture("site_map_other.json");
    delete raw.site.no_readings;
    delete raw.site.other_rigs;
    delete raw.site.presence_line;
    mount(raw);
    expect(sideTexts()).toEqual([]);
  });
});
