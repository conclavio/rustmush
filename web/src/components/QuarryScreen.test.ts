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
import { QuestsStore } from "$lib/stores/quests.svelte";

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

  it("highlights matching quest mineral readings and prefills the next mining action", () => {
    const raw = fixture("site_map_other.json");
    raw.site.family = "carbon";
    raw.site.readings = [
      { target: "B", line: "B · 140 m N — rich carbon_ore pocket" },
    ];
    const quarry = new QuarryStore();
    quarry.applySite(raw, Date.now());
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "Run +tv/1 scan go.", checked: true, live_ready: true },
            { id: "sound_mineral_pocket", title: "Sound pocket", instructions: "Run +tv/1 sound go.", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: false, live_ready: false },
          ],
        },
      ],
    });
    const focusComposer = vi.fn();
    const app = { quarry, quests, focusComposer, refreshQuarry: vi.fn() } as unknown as GameApp;
    render(QuarryScreen, { props: { app } });

    const nextBtn = screen.getByRole("button", { name: /Next: Sound Pocket/i });
    nextBtn.click();
    expect(focusComposer).toHaveBeenCalledWith("+tv sound go");

    const readingBtn = screen.getByRole("button", { name: /\[QUEST\] B · 140 m N — rich carbon_ore pocket/i });
    expect(readingBtn.classList.contains("quest-match")).toBe(true);

    const chartBtn = screen.getByRole("button", { name: /Ground Chart/i });
    chartBtn.click();
    expect(focusComposer).toHaveBeenCalledWith("+chart/ground");

    const recoverBtn = screen.getByRole("button", { name: /Recover Rig/i });
    recoverBtn.click();
    expect(focusComposer).toHaveBeenCalledWith("+tv/1 recover go");
  });

  it("shows prospecting prep chips before first scan and switches Next button to Recover Rig after mining", () => {
    const raw = fixture("site_map_other.json");
    const quarry = new QuarryStore();
    quarry.applySite(raw, Date.now());
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "Run +tv/1 scan go.", checked: false, live_ready: false },
            { id: "sound_mineral_pocket", title: "Sound pocket", instructions: "Run +tv/1 sound go.", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: false, live_ready: false },
          ],
        },
      ],
    });
    const focusComposer = vi.fn();
    const app = { quarry, quests, focusComposer, refreshQuarry: vi.fn() } as unknown as GameApp;
    const first = render(QuarryScreen, { props: { app } });

    screen.getByRole("button", { name: /^License$/i }).click();
    expect(focusComposer).toHaveBeenCalledWith("+license");
    screen.getByRole("button", { name: /^Lease Rig$/i }).click();
    expect(focusComposer).toHaveBeenCalledWith("+tv/lease");
    first.unmount();

    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "Run +tv/1 scan go.", checked: true, live_ready: true },
            { id: "sound_mineral_pocket", title: "Sound pocket", instructions: "Run +tv/1 sound go.", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "Run +tv/1 mine go.", checked: true, live_ready: true },
          ],
        },
      ],
    });
    render(QuarryScreen, { props: { app } });
    screen.getByRole("button", { name: /Next: Recover Rig/i }).click();
    expect(focusComposer).toHaveBeenCalledWith("+tv recover go");
  });

  it("offers 1-click Plan button for matching carbon landing site on Ground Chart and Return button after mining", () => {
    const rawGround = fixture("body_ground.json") as unknown as {
      ground: { sites: Array<{ venue: string; site: string; best: string | null; best_line: string }> };
    };
    const commons = rawGround.ground.sites.find((s) => s.venue === "commons");
    if (commons) {
      commons.best = "carbon_ore";
      commons.best_line = "Your readings: carbon_ore rich.";
    }
    const quarry = new QuarryStore();
    quarry.mode = "ground";
    quarry.applyGround(rawGround);
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: false, live_ready: false },
          ],
        },
      ],
    });
    const focusComposer = vi.fn();
    const app = { quarry, quests, focusComposer, refreshQuarry: vi.fn() } as unknown as GameApp;
    const first = render(QuarryScreen, { props: { app } });

    const planBtn = screen.getByRole("button", { name: /Plan: Freeport Commons/i });
    planBtn.click();
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan commons");
    first.unmount();

    // Once carbon_ore is mined, pill offers 1-click Return to Ceres Freeport
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          ready_to_finish: true,
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: true, live_ready: true },
          ],
        },
      ],
    });
    render(QuarryScreen, { props: { app } });
    const retBtn = screen.getByRole("button", { name: /Return: Ceres Freeport/i });
    retBtn.click();
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan ceres");
  });
});
