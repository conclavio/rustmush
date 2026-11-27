// @vitest-environment jsdom

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import ChartScreen from "./ChartScreen.svelte";
import type { GameApp } from "$lib/game.svelte";
import { ChartStore } from "$lib/stores/chart.svelte";
import { QuestsStore } from "$lib/stores/quests.svelte";

function skyChartFixture(): unknown {
  return JSON.parse(
    readFileSync(resolve(process.cwd(), "src/lib/fixtures/chart/sky_chart.json"), "utf8"),
  );
}

describe("ChartScreen quest destination guidance", () => {
  it("stays clean without an active destination quest and highlights the target body, route, and passage row when active", async () => {
    const chart = new ChartStore();
    chart.applyKeyframe(skyChartFixture());
    const quests = new QuestsStore();
    const focusComposer = vi.fn();
    const tacsim = { selectedDestination: "ceres" };
    const app = { chart, quests, tacsim, focusComposer } as unknown as GameApp;

    const { container } = render(ChartScreen, { props: { app } });
    expect(container.querySelector(".quest-dest-banner")).toBeNull();
    expect(container.querySelector("[data-quest-body]")).toBeNull();

    // Activate Step 3 quest heading to Stickney Yards (+ship/plan stickney)
    quests.applyFrame({
      schema: 1,
      room_id: "#10",
      room_name: "Ceres Berth",
      room_markers: [],
      available: [],
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_deliver_metals",
          title: "Deliver Metals to Stickney",
          category: "guided",
          next_step: "Plot passage to Stickney Yards (+ship/plan stickney)",
          checkpoints: [
            {
              id: "sail_stickney",
              title: "Deliver 5 metals to Stickney Yards",
              instructions: "Run +ship/plan stickney and +ship/order depart stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      completed: [],
    });

    const banner = await screen.findByRole("status", { name: "active quest destination" });
    expect(banner.textContent).toContain("[QUEST]");
    expect(banner.textContent).toContain("Stickney Yards");
    expect(banner.textContent).toContain("Deliver Metals to Stickney");

    // Primary planet (Mars) gets a golden SVG objective reticle, and Stickney route + passage row highlight
    expect(container.querySelector('[data-quest-body="mars"]')).not.toBeNull();
    expect(container.querySelector("[data-quest-route]")).not.toBeNull();
    expect(container.querySelector("li.quest-row")?.textContent).toContain("[QUEST]");

    // 1-click Plan button selects stickney in TacsimStore and prefills `+ship/plan stickney`
    const planBtn = screen.getByRole("button", { name: "Plan +ship/plan stickney" });
    await fireEvent.click(planBtn);
    expect(tacsim.selectedDestination).toBe("stickney");
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan stickney");
  });
});
