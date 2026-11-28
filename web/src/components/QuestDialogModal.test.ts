// @vitest-environment jsdom

import { fireEvent, render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import QuestDialogModal from "./QuestDialogModal.svelte";
import type { GameApp } from "$lib/game.svelte";
import { GUIDED_TOUR_KEYS, QuestsStore } from "$lib/stores/quests.svelte";

describe("QuestDialogModal guided corridor tour roadmap", () => {
  it("renders all 7 labeled roadmap steps and opens quest dialog when clicking an active or available step", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: "Registry & Co-op Desk",
      room_markers: [],
      completed: [
        { key: GUIDED_TOUR_KEYS[0], title: "First Papers at the Registry" },
      ],
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          introduction: "Buy 10 provisions and 5t propellant.",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Ilse Oduya-Brandt",
          reward_summary: "Guided Tour Step 2/7",
          next_step: "Buy 10 provisions",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy 10 provisions",
              instructions: "Visit Co-op Chandlery.",
              room_entry: "room.c08_chandlery",
              room_name: "Co-op Chandlery",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          introduction: "Haul 5t metals to Stickney.",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Jonas Achterberg-Nkemelu",
          reward_summary: "Guided Tour Step 3/7",
          accept_cmd: "+quest/accept #18 tok3",
          checkpoints: [],
        },
      ],
    });
    quests.openLog("active");

    const app = {
      quests,
      tacsim: { trade: null },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    render(QuestDialogModal, { app, onCmd: vi.fn() });

    const step1 = screen.getByText("1. Co-op Charter").closest(".tour-step");
    const step2Btn = screen.getByRole("button", { name: /2\. Locker & Tanks/ });
    const step3Btn = screen.getByRole("button", { name: /3\. Metals Haul/ });
    const step7 = screen.getByText("7. Sell Mined Ore").closest(".tour-step");

    expect(step1?.classList.contains("done")).toBe(true);
    expect(step2Btn.classList.contains("current")).toBe(true);
    expect(step3Btn.classList.contains("avail")).toBe(true);
    expect(step7?.classList.contains("upcoming")).toBe(true);

    // Clicking an available step opens its quest dialog directly
    await fireEvent.click(step3Btn);
    expect(quests.dialogOpen).toBe(true);
    expect(quests.dialogQuestKey).toBe(GUIDED_TOUR_KEYS[2]);
    expect(screen.getByRole("dialog", { name: "Quest dialog" })).toBeTruthy();
  });

  it("displays live hold progress and Step 5 refit hold check inside Quest Log and NPC Quest Dialog", () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#17",
      room_name: "Exchange Hall",
      active: [
        {
          key: GUIDED_TOUR_KEYS[2],
          title: "Metals Down the Corridor",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Jonas Achterberg-Nkemelu",
          next_step: "Buy 5t metals and sail to Stickney",
          checkpoints: [
            {
              id: "deliver_metals",
              title: "Buy 5 tonnes of metals at Ceres and deliver to Stickney",
              instructions: "Buy 5t metals at Exchange and +market/sell at Stickney.",
              checked: false,
              live_ready: false,
            },
          ],
        },
        {
          key: GUIDED_TOUR_KEYS[4],
          title: "Refit the Dray for Surface Mining",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Rosalind Qureshi-Mbeki",
          next_step: "Run +coop/refit prospector",
          checkpoints: [
            {
              id: "refit_dray_p",
              title: "Refit your Dray into a Dray-P prospector",
              instructions: "Empty hold and run +coop/refit prospector.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [],
      completed: [],
    });
    quests.openLog("active");

    const app = {
      quests,
      tacsim: {
        trade: {
          hullId: "#99",
          stowage: {
            cargoKg: 2000,
            propellantKg: 10000,
            provisionsCrewDays: 8,
            cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 2 }],
          },
        },
      },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    const first = render(QuestDialogModal, { app, onCmd: vi.fn() });
    expect(screen.getByText(/METALS: 2\/5 u/i)).toBeTruthy();
    expect(screen.getByText(/\[REFIT CHECK\]/i)).toBeTruthy();
    expect(screen.getByText(/Stores 8d/i)).toBeTruthy();
    first.unmount();

    // Open NPC Quest Dialog for Step 3 with 5/5 metals loaded -> shows [HOLD READY]
    (app.tacsim as unknown as { trade: { hullId: string; stowage: Record<string, unknown> } }).trade.stowage = {
      cargoKg: 5000,
      propellantKg: 10000,
      provisionsCrewDays: 8,
      cargoLines: [{ good: "tacsim:metals", label: "Metals", qty: 5 }],
    };
    quests.openQuestDialog(GUIDED_TOUR_KEYS[2]!);
    render(QuestDialogModal, { app, onCmd: vi.fn() });
    expect(screen.getByText(/\[HOLD READY\]/i)).toBeTruthy();
    expect(screen.getByText(/METALS: 5\/5 u/i)).toBeTruthy();
  });

  it("shows Optional · Restartable indicator, switches to Completed tab from done roadmap step, and celebrates 7/7 completion", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: "Registry & Co-op Desk",
      active: [
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          category: "guided",
          giver_name: "Maren Halvorsen",
          turn_in_name: "Ilse Oduya-Brandt",
          allow_restart: true,
          reward_summary: "Guided Tour Step 1/7",
          next_step: "Enroll with Ilse",
          checkpoints: [],
        },
      ],
      available: [],
      completed: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          turn_in_name: "Ilse Oduya-Brandt",
          completion: "Tanks topped and galley stocked.",
          reward_summary: "Guided Tour Step 2/7",
        },
      ],
    });
    quests.openLog("active");
    const app = { quests, tacsim: { trade: null }, focusComposer: vi.fn() } as unknown as GameApp;

    const first = render(QuestDialogModal, { app, onCmd: vi.fn() });
    expect(screen.getByText(/Guided · Restartable/i)).toBeTruthy();
    // Clicking a completed roadmap pill switches Quest Log to Completed tab
    const donePill = screen.getByRole("button", { name: /2\. Locker & Tanks/ });
    await fireEvent.click(donePill);
    expect(quests.logTab).toBe("completed");
    expect(screen.getByText("Step 2/7")).toBeTruthy();
    expect(screen.getByText(/Reward: Guided Tour Step 2\/7/)).toBeTruthy();
    first.unmount();

    // Active quest dialog also shows Optional · Restartable footnote
    quests.openQuestDialog(GUIDED_TOUR_KEYS[0]!);
    const second = render(QuestDialogModal, { app, onCmd: vi.fn() });
    expect(screen.getByText(/Optional · Restartable/i)).toBeTruthy();
    second.unmount();

    // All 7/7 completed -> Completed tab shows Corridor Captain celebration banner
    quests.applyFrame({
      schema: 1,
      active: [],
      available: [],
      completed: GUIDED_TOUR_KEYS.map((key, idx) => ({
        key,
        title: `Guided Step ${idx + 1}`,
        turn_in_name: "Corridor Host",
        completion: `Completed step ${idx + 1}.`,
        reward_summary: `Tour Step ${idx + 1}/7`,
      })),
    });
    quests.openLog("completed");
    render(QuestDialogModal, { app, onCmd: vi.fn() });
    const banner = screen.getByRole("status", { name: "Guided corridor tour complete" });
    expect(banner.textContent).toContain("Guided Corridor Tour Complete (7/7) — Corridor Captain");
  });

  it("advances Step 2 dual-item hold readiness past loaded provisions and shows secondary OK status", () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#17",
      room_name: "Exchange Hall",
      active: [
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_name: "Ilse Oduya-Brandt",
          next_step: "Buy 10 provisions and 5t propellant",
          checkpoints: [
            {
              id: "buy_provisions",
              title: "Buy 10 provisions",
              instructions: "Buy 10 provisions at the Chandlery.",
              checked: false,
              live_ready: false,
            },
            {
              id: "buy_propellant",
              title: "Buy 5t propellant",
              instructions: "Buy 5 tonnes of propellant at the Exchange.",
              checked: false,
              live_ready: false,
            },
          ],
        },
      ],
      available: [],
      completed: [],
    });
    quests.openQuestDialog(GUIDED_TOUR_KEYS[1]!);

    const app = {
      quests,
      tacsim: {
        trade: {
          hullId: "#99",
          stowage: {
            cargoKg: 0,
            propellantKg: 1000,
            provisionsCrewDays: 10,
            cargoLines: [],
          },
        },
      },
      focusComposer: vi.fn(),
    } as unknown as GameApp;

    const first = render(QuestDialogModal, { app, onCmd: vi.fn() });
    const strip = screen.getByLabelText("quest hold progress");
    expect(strip.textContent).toContain("[HOLD]");
    expect(strip.textContent).toContain("PROPELLANT: 1/5 t");
    expect(strip.textContent).toContain("PROVISIONS 10/10d OK");
    first.unmount();

    // Also verify Quest Log active card shows the same multi-item summary once both are ready
    (app.tacsim as unknown as { trade: { hullId: string; stowage: Record<string, unknown> } }).trade.stowage = {
      cargoKg: 0,
      propellantKg: 5000,
      provisionsCrewDays: 10,
      cargoLines: [],
    };
    quests.openLog("active");
    render(QuestDialogModal, { app, onCmd: vi.fn() });
    const logStrip = screen.getByLabelText("quest hold progress");
    expect(logStrip.textContent).toContain("[HOLD READY]");
    expect(logStrip.textContent).toContain("PROVISIONS: 10/10 d");
    expect(logStrip.textContent).toContain("PROPELLANT 5/5t OK");
  });
});
