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

  it("highlights between-steps Next Guided Offer port when off-station and shows Ground Chart jump for Step 6", async () => {
    const chart = new ChartStore();
    chart.applyKeyframe(skyChartFixture());
    const quests = new QuestsStore();
    const focusComposer = vi.fn();
    const switchScreen = vi.fn().mockReturnValue(true);
    const quarry = { mode: "site", family: "volatiles" };
    const tacsim = { selectedDestination: "ceres" };
    const app = {
      chart,
      quests,
      tacsim,
      quarry,
      switchScreen,
      focusComposer,
    } as unknown as GameApp;

    // Between steps 3 & 4 while reading sky chart from Ceres -> highlights Stickney Yards for Jonas
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: "tacsim.quests.guided_onboarding/quest.guide_01_coop_enroll", title: "S1" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_02_provisions_fuel", title: "S2" },
        { key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery", title: "S3" },
      ],
      active: [],
      available: [],
    });

    const first = render(ChartScreen, { props: { app } });
    const banner = await screen.findByRole("status", { name: "active quest destination" });
    expect(banner.textContent).toContain("Stickney Yards");
    expect(banner.textContent).toContain("Next Guided (4/7)");
    await fireEvent.click(screen.getByRole("button", { name: "Plan +ship/plan stickney" }));
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan stickney");
    first.unmount();

    // Active Step 6 surface mining -> offers 1-click Ground Chart (CARBON) jump
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          introduction: "Choose an open Ceres landing site such as Freeport Commons.",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: false, live_ready: false },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: false, live_ready: false },
          ],
        },
      ],
      available: [],
      completed: [],
    });
    render(ChartScreen, { props: { app } });
    const groundBtn = screen.getByRole("button", { name: /Ground Chart \(CARBON\)/i });
    await fireEvent.click(groundBtn);
    expect(quarry.mode).toBe("ground");
    expect(quarry.family).toBe("carbon");
    expect(switchScreen).toHaveBeenCalledWith("quarry");
  });

  it("sorts the quest destination passage row first and prefills +ship/plan when clicked", async () => {
    const raw = skyChartFixture() as {
      chart: { routes: Array<{ to: string; venue: string; body: string }> };
    };
    // Ensure a non-stickney route precedes stickney in raw server order
    const stickneyIdx = raw.chart.routes.findIndex(
      (r) => r.venue.toLowerCase() === "stickney" || r.to.toLowerCase().includes("stickney"),
    );
    if (stickneyIdx === 0 && raw.chart.routes.length > 1) {
      const [first] = raw.chart.routes.splice(0, 1);
      raw.chart.routes.push(first!);
    }

    const chart = new ChartStore();
    chart.applyKeyframe(raw);
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney",
              instructions: "+ship/plan stickney",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const focusComposer = vi.fn();
    const tacsim = { selectedDestination: "ceres" };
    const app = { chart, quests, tacsim, focusComposer } as unknown as GameApp;

    const { container } = render(ChartScreen, { props: { app } });
    const rows = [...container.querySelectorAll("ul.passage-list li")];
    expect(rows.length).toBeGreaterThanOrEqual(1);
    expect(rows[0]!.classList.contains("quest-row")).toBe(true);
    expect(rows[0]!.textContent).toContain("[QUEST]");

    const btn = rows[0]!.querySelector("button.passage-btn") as HTMLButtonElement;
    await fireEvent.click(btn);
    expect(tacsim.selectedDestination).toBe("stickney");
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan stickney");
  });

  it("displays Step 6/7 badge, highlights Ceres during surface mining, and transitions to Ceres return post-mine", async () => {
    const chart = new ChartStore();
    chart.applyKeyframe(skyChartFixture());
    const quests = new QuestsStore();
    const focusComposer = vi.fn();
    const tacsim = { selectedDestination: "stickney" };
    const app = { chart, quests, tacsim, focusComposer } as unknown as GameApp;

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
    const { container } = render(ChartScreen, { props: { app } });
    const banner = await screen.findByRole("status", { name: "active quest destination" });
    expect(banner.textContent).toContain("Step 6/7");
    expect(container.querySelector('[data-quest-body="ceres"]')).not.toBeNull();

    // Once mining completes -> flips to Return Destination: Ceres Freeport (+ship/plan ceres)
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Surface scan", instructions: "+tv/1 scan go", checked: true, live_ready: true },
            { id: "sound_pocket", title: "Sound pocket", instructions: "+tv/1 sound go", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", instructions: "+tv/1 mine go", checked: true, live_ready: true },
          ],
        },
      ],
    });
    const planBtn = await screen.findByRole("button", { name: "Plan +ship/plan ceres" });
    await fireEvent.click(planBtn);
    expect(tacsim.selectedDestination).toBe("ceres");
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan ceres");
  });

  it("shows live pre-flight Fuel/Stores readiness before departure and flips to [QUEST ARRIVED] once at destination port", async () => {
    const raw = skyChartFixture() as { chart: { vantage: { port: string; body: string } } };
    const chart = new ChartStore();
    chart.applyKeyframe(raw);
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          next_room_entry: "room.p07_receiving",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney Yards",
              instructions: "+ship/plan stickney and sell metals.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const focusComposer = vi.fn();
    const app1 = {
      chart,
      quests,
      tacsim: {
        selectedDestination: "ceres",
        isAboardShip: true,
        ambient: {
          ownStowage: {
            cargoKg: 5000,
            propellantKg: 0,
            provisionsCrewDays: 0,
            cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5 }],
          },
        },
      },
      focusComposer,
    } as unknown as GameApp;

    // 1. At Ceres heading to Stickney with 0 fuel/stores -> [CHECK BEFORE UNDOCK]
    const r1 = render(ChartScreen, { props: { app: app1 } });
    const pf = await screen.findByLabelText("chart sailing preflight");
    expect(pf.textContent).toContain("[CHECK BEFORE UNDOCK]");
    expect(pf.textContent).toContain("Fuel 0t");
    r1.unmount();

    // 2. Vantage now at Stickney Yards aboard ship -> [QUEST ARRIVED] + Step Ashore (+ship/leave)
    raw.chart.vantage.port = "Stickney Yards";
    raw.chart.vantage.body = "stickney";
    chart.applyKeyframe(raw);
    render(ChartScreen, { props: { app: app1 } });
    const banner = await screen.findByRole("status", { name: "active quest destination" });
    expect(banner.textContent).toContain("[QUEST ARRIVED] Step 3/7");
    expect(banner.textContent).toContain("At Stickney Yards");
    expect(screen.queryByLabelText("chart sailing preflight")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: /Step Ashore \(\+ship\/leave\)/i }));
    expect(focusComposer).toHaveBeenCalledWith("+ship/leave");
  });

  it("shows cargo-aware pre-flight and guards Step 6 post-mine ChartScreen with [RECOVER RIG FIRST] until ore is stowed", async () => {
    const raw = skyChartFixture() as { chart: { vantage: { port: string; body: string } } };
    const chart = new ChartStore();
    chart.applyKeyframe(raw);
    const quests = new QuestsStore();
    const focusComposer = vi.fn();

    // 1. Step 3 with fuel & stores OK but 0/5 metals -> [CHECK BEFORE UNDOCK] with ✗ metals 0/5
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_03_metals_delivery",
          title: "Metals Down the Corridor",
          next_room_entry: "room.p07_receiving",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Deliver 5t metals to Stickney Yards",
              instructions: "Buy 5t metals (+market/buy metals 5) then +ship/plan stickney and sell.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
    });
    const ownStowage = {
      cargoKg: 0,
      propellantKg: 10_000,
      provisionsCrewDays: 10,
      cargoLines: [] as Array<{ good: string; label: string; qty: number }>,
    };
    const app = {
      chart,
      quests,
      tacsim: {
        selectedDestination: "ceres",
        ambient: { ownStowage, settledPad: false },
      },
      focusComposer,
    } as unknown as GameApp;

    const r1 = render(ChartScreen, { props: { app } });
    const pfMissing = await screen.findByLabelText("chart sailing preflight");
    expect(pfMissing.textContent).toContain("[CHECK BEFORE UNDOCK]");
    expect(pfMissing.textContent).toContain("✗ metals 0/5");
    r1.unmount();

    ownStowage.cargoKg = 5000;
    ownStowage.cargoLines = [{ good: "tacsim:metals", label: "Metals", qty: 5 }];
    const r2 = render(ChartScreen, { props: { app } });
    const pfOk = await screen.findByLabelText("chart sailing preflight");
    expect(pfOk.textContent).toContain("[PRE-FLIGHT OK]");
    expect(pfOk.textContent).toContain("✓ metals 5/5");
    r2.unmount();

    // 2. Step 6 post-mine with 0t ore stowed (even with Ceres vantage & settledPad) -> [RECOVER RIG FIRST] (+tv/1 recover go)
    ownStowage.cargoKg = 0;
    ownStowage.cargoLines = [];
    (app.tacsim.ambient as { settledPad: boolean }).settledPad = true;
    quests.applyFrame({
      schema: 1,
      active: [
        {
          key: "tacsim.quests.guided_onboarding/quest.guide_06_surface_mining",
          title: "Scan, Sound, and Mine a Landing Site",
          next_room_entry: "room.c07_exchange",
          checkpoints: [
            { id: "scan_landing_site", title: "Scan site", checked: true, live_ready: true },
            { id: "sound_pocket", title: "Sound pocket", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", checked: true, live_ready: true },
          ],
        },
      ],
    });
    const r3 = render(ChartScreen, { props: { app } });
    const bRecover = await screen.findByRole("status", { name: "active quest destination" });
    expect(bRecover.textContent).toContain("[RECOVER RIG FIRST] Step 6/7");
    expect(screen.queryByRole("button", { name: "Plan +ship/plan ceres" })).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: /Recover Rig \(\+tv\/1 recover go\)/i }));
    expect(focusComposer).toHaveBeenCalledWith("+tv/1 recover go");
    r3.unmount();

    // 3. Rig recovered (1t carbon_ore stowed) -> [ORE HOLD READY] Return to: Ceres Freeport + Plan +ship/plan ceres
    focusComposer.mockClear();
    ownStowage.cargoKg = 1000;
    ownStowage.cargoLines = [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 1 }];
    render(ChartScreen, { props: { app } });
    const bReady = await screen.findByRole("status", { name: "active quest destination" });
    expect(bReady.textContent).toContain("[ORE HOLD READY] Step 6/7");
    expect(bReady.textContent).toContain("Return to: Ceres Freeport");
    await fireEvent.click(screen.getByRole("button", { name: "Plan +ship/plan ceres" }));
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan ceres");
  });
});
