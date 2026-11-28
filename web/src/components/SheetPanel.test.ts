// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import SheetPanel from "./SheetPanel.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";

describe("SheetPanel guided corridor tour summary", () => {
  it("renders Tour progress badge, clickable 7-step roadmap, and between-steps Next Guided locator", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "First Papers" },
        { key: GUIDED_TOUR_KEYS[1], title: "Stock Locker" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          next_step: "Buy 5t metals at Exchange",
          checkpoints: [{ id: "deliver_metals", title: "Deliver 5t metals", checked: false, live_ready: false }],
        },
      ],
      available: [],
    });

    const app = {
      sheet: {
        loaded: true,
        unavailable: false,
        approval: null,
        schema: { sections: [] },
        data: {},
      },
      quests,
      refreshSheet: vi.fn(),
    } as unknown as GameApp;

    const first = render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Tour 2\/7/i)).toBeTruthy();
    const step3Btn = screen.getByRole("button", { name: /3\. Metals Haul/i });
    await fireEvent.click(step3Btn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe(GUIDED_TOUR_KEYS[2]);
    first.unmount();

    // Between steps (Steps 1–3 complete, no active or room offers) -> shows Next Guided (4/7) Jonas
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "S1" },
        { key: GUIDED_TOUR_KEYS[1], title: "S2" },
        { key: GUIDED_TOUR_KEYS[2], title: "S3" },
      ],
      active: [],
      available: [],
    });
    render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Tour 3\/7/i)).toBeTruthy();
    expect(screen.getByText(/Next Guided \(4\/7\):/i)).toBeTruthy();
    expect(screen.getByText(/Jonas Achterberg-Nkemelu/i)).toBeTruthy();
  });

  it("clicks completed roadmap step to Completed log, shows Step n/7, secondary turn-in alert, and guided offer priority", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [{ key: GUIDED_TOUR_KEYS[0], title: "First Papers" }],
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker",
          next_step: "Buy 10 provisions at Chandlery",
          can_finish_now: false,
          checkpoints: [{ id: "buy_prov", title: "Buy 10 provisions", checked: false, live_ready: false }],
        },
        {
          key: "quest.side_ready",
          title: "Dockside Ledger Check",
          next_step: "Turn in here",
          can_finish_now: true,
          ready_to_finish: true,
          checkpoints: [{ id: "check", title: "Check", checked: true, live_ready: true }],
        },
      ],
      available: [
        {
          key: "quest.side_offer",
          title: "Barge Sweeping",
          category: "side",
          giver_name: "Dock Clerk",
        },
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          category: "guided",
          giver_name: "Ilse Oduya-Brandt",
        },
      ],
    });
    quests.track(GUIDED_TOUR_KEYS[1]!);

    const app = {
      sheet: {
        loaded: true,
        unavailable: false,
        approval: null,
        schema: { sections: [] },
        data: {},
      },
      quests,
      refreshSheet: vi.fn(),
    } as unknown as GameApp;

    const first = render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Step 2\/7/i)).toBeTruthy();

    // Clicking completed Step 1 opens the Completed tab of the Quest Log
    const step1Btn = screen.getByRole("button", { name: /1\. Co-op Charter/i });
    await fireEvent.click(step1Btn);
    expect(quests.logOpen).toBe(true);
    expect(quests.logTab).toBe("completed");

    // Secondary ready turn-in alert opens that quest's dialog directly
    const readyBtn = screen.getByRole("button", { name: /\[\?\] Ready to turn in: Dockside Ledger Check/i });
    await fireEvent.click(readyBtn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe("quest.side_ready");

    // Room offers badge while active
    expect(screen.getByRole("button", { name: /\[!\] 2 quest offers here/i })).toBeTruthy();
    first.unmount();

    // Idle with multiple offers prioritizes the guided offer with (+1 more)
    quests.applyFrame({
      schema: 1,
      completed: [{ key: GUIDED_TOUR_KEYS[0], title: "First Papers" }],
      active: [],
      available: [
        {
          key: "quest.side_offer",
          title: "Barge Sweeping",
          category: "side",
          giver_name: "Dock Clerk",
        },
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          category: "guided",
          giver_name: "Ilse Oduya-Brandt",
        },
      ],
    });
    render(SheetPanel, { props: { app } });
    const offerBtn = screen.getByRole("button", {
      name: /\[!\] Ilse Oduya-Brandt offers: Metals Down the Corridor \(\+1 more\)/i,
    });
    await fireEvent.click(offerBtn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe(GUIDED_TOUR_KEYS[2]);
  });

  it("displays live hold readiness, cross-port pre-flight check, and Corridor Captain Jobs Board shortcut on SheetPanel", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [],
      active: [
        {
          key: GUIDED_TOUR_KEYS[6],
          title: "Deliver Your Mined Ore to Market",
          next_step: "Fly to Stickney Yards and sell carbon_ore",
          checkpoints: [
            {
              id: "sell_carbon_ore",
              title: "Sell 1t carbon_ore at Stickney Yards",
              instructions: "Fly to Stickney Yards (+ship/plan stickney) and +market/sell carbon_ore.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [],
    });

    const focusComposer = vi.fn();
    const app = {
      sheet: {
        loaded: true,
        unavailable: false,
        approval: null,
        schema: { sections: [] },
        data: {},
      },
      quests,
      tacsim: {
        ambient: {
          ownStowage: {
            cargoKg: 2000,
            propellantKg: 12_000,
            provisionsCrewDays: 0,
            cargoLines: [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 2 }],
          },
        },
      },
      refreshSheet: vi.fn(),
      focusComposer,
    } as unknown as GameApp;

    const first = render(SheetPanel, { props: { app } });
    const holdEl = screen.getByLabelText("sheet quest hold readiness");
    expect(holdEl.textContent).toContain("[HOLD READY]");
    expect(holdEl.textContent).toContain("CARBON ORE: 2/1");

    const pfEl = screen.getByLabelText("sheet quest sailing preflight");
    expect(pfEl.textContent).toContain("[CHECK BEFORE UNDOCK]");
    expect(pfEl.textContent).toContain("✓ Propellant 12t");
    expect(pfEl.textContent).toContain("✗ Provisions 0d");
    expect(pfEl.textContent).toContain("→ Stickney Yards");
    first.unmount();

    // All 7 steps complete -> Corridor Captain mastery + 1-click Jobs Board shortcut
    quests.applyFrame({
      schema: 1,
      completed: GUIDED_TOUR_KEYS.map((key, i) => ({ key, title: `Step ${i + 1}` })),
      active: [],
      available: [],
    });
    render(SheetPanel, { props: { app } });
    expect(screen.getByText(/Corridor Captain \(7\/7\)/i)).toBeTruthy();
    const jobsBtn = screen.getByRole("button", { name: "Jobs Board (+market/jobs)" });
    await fireEvent.click(jobsBtn);
    expect(focusComposer).toHaveBeenCalledWith("+market/jobs");
  });

  it("shows cargo-aware sailing preflight, stowage-aware Next Command, and Step 6 rig recovery guard on SheetPanel", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "S1" },
        { key: GUIDED_TOUR_KEYS[1], title: "S2" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          next_step: "Buy 5t metals at Ceres Exchange and deliver to Stickney Yards",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy and deliver 5t metals to Stickney Yards",
              instructions: "Buy 5t metals (+market/buy metals 5) then sail to Stickney Yards (+ship/plan stickney) and sell.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [],
    });

    const focusComposer = vi.fn();
    const ownStowage = {
      cargoKg: 0,
      propellantKg: 10_000,
      provisionsCrewDays: 10,
      cargoLines: [] as Array<{ good: string; label: string; qty: number }>,
    };
    const app = {
      sheet: {
        loaded: true,
        unavailable: false,
        approval: null,
        schema: { sections: [] },
        data: {},
      },
      quests,
      tacsim: {
        ambient: { ownStowage },
      },
      refreshSheet: vi.fn(),
      focusComposer,
    } as unknown as GameApp;

    // 1. Step 3 with fuel & provisions OK but 0/5 metals -> [CHECK BEFORE UNDOCK] + Buy Cargo (+market/list)
    const s3Empty = render(SheetPanel, { props: { app } });
    const pfEmpty = screen.getByLabelText("sheet quest sailing preflight");
    expect(pfEmpty.textContent).toContain("[CHECK BEFORE UNDOCK]");
    expect(pfEmpty.textContent).toContain("✗ metals 0/5");
    const buyBtn = screen.getByRole("button", { name: /Next: Buy Cargo \(\+market\/list\)/i });
    await fireEvent.click(buyBtn);
    expect(focusComposer).toHaveBeenCalledWith("+market/list");
    s3Empty.unmount();

    // 2. Step 3 with 5t metals stowed -> [PRE-FLIGHT OK] + Plan Stickney Yards (+ship/plan stickney)
    focusComposer.mockClear();
    ownStowage.cargoKg = 5000;
    ownStowage.cargoLines = [{ good: "tacsim:metals", label: "Metals", qty: 5 }];
    const s3Loaded = render(SheetPanel, { props: { app } });
    const pfLoaded = screen.getByLabelText("sheet quest sailing preflight");
    expect(pfLoaded.textContent).toContain("[PRE-FLIGHT OK]");
    expect(pfLoaded.textContent).toContain("✓ metals 5/5");
    const sailBtn = screen.getByRole("button", { name: /Next: Plan Stickney Yards \(\+ship\/plan stickney\)/i });
    await fireEvent.click(sailBtn);
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan stickney");
    s3Loaded.unmount();

    // 3. Step 6 post-mine with 0t carbon_ore in ship hold -> [RECOVER RIG FIRST] + Recover Rig (+tv/1 recover go)
    focusComposer.mockClear();
    ownStowage.cargoKg = 0;
    ownStowage.cargoLines = [];
    quests.applyFrame({
      schema: 1,
      completed: GUIDED_TOUR_KEYS.slice(0, 5).map((key, i) => ({ key, title: `S${i + 1}` })),
      active: [
        {
          key: GUIDED_TOUR_KEYS[5],
          title: "Prospect the Surface",
          next_room_entry: "room.c07_exchange",
          next_step: "Recover rig and return carbon_ore to Rosalind at Ceres Exchange",
          checkpoints: [
            { id: "buy_license", title: "Buy prospecting license", checked: true, live_ready: true },
            { id: "lease_rig", title: "Lease mining rig", checked: true, live_ready: true },
            { id: "scan_site", title: "Scan site", checked: true, live_ready: true },
            { id: "sound_pocket", title: "Sound pocket", checked: true, live_ready: true },
            { id: "mine_carbon_ore", title: "Mine 1t carbon_ore", checked: true, live_ready: true },
            { id: "return_exchange", title: "Return to Ceres Exchange", checked: false, live_ready: false },
          ],
        },
      ],
      available: [],
    });
    const s6Recover = render(SheetPanel, { props: { app } });
    const holdRecover = screen.getByLabelText("sheet quest hold readiness");
    expect(holdRecover.textContent).toContain("[RECOVER RIG FIRST]");
    expect(holdRecover.textContent).toContain("+tv/1 recover go");
    const recBtn = screen.getByRole("button", { name: /Next: Recover Rig \(\+tv\/1 recover go\)/i });
    await fireEvent.click(recBtn);
    expect(focusComposer).toHaveBeenCalledWith("+tv/1 recover go");
    s6Recover.unmount();

    // 4. Step 6 after rig recovery (1t carbon_ore stowed) -> [ORE HOLD READY] + Plan Ceres Freeport (+ship/plan ceres)
    focusComposer.mockClear();
    ownStowage.cargoKg = 1000;
    ownStowage.cargoLines = [{ good: "tacsim:carbon_ore", label: "Carbon Ore", qty: 1 }];
    render(SheetPanel, { props: { app } });
    const holdReady = screen.getByLabelText("sheet quest hold readiness");
    expect(holdReady.textContent).toContain("[ORE HOLD READY]");
    const retBtn = screen.getByRole("button", { name: /Next: Plan Ceres Freeport \(\+ship\/plan ceres\)/i });
    await fireEvent.click(retBtn);
    expect(focusComposer).toHaveBeenCalledWith("+ship/plan ceres");
  });
});
