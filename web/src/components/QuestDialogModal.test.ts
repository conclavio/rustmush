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

  it("updates header crest and resets abandon confirmation when switching multi-quest NPC tabs, and supports permanent decline in Quest Log Available tab", async () => {
    const quests = new QuestsStore();
    quests.applyFrame({
      schema: 1,
      room_id: "#15",
      room_name: "Registry & Co-op Desk",
      room_markers: [
        {
          npc_id: "#18",
          npc_name: "Ilse Oduya-Brandt",
          marker: "turn_in",
          symbol: "?",
          quest_key: GUIDED_TOUR_KEYS[0],
          quest_title: "First Papers at the Registry",
          category: "guided",
        },
      ],
      active: [
        {
          key: "side.in_progress",
          title: "In-Progress Favor",
          giver_id: "#18",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          can_finish_now: false,
          ready_to_finish: false,
          abandon_cmd: "+quest/abandon side.in_progress",
          checkpoints: [],
        },
        {
          key: GUIDED_TOUR_KEYS[0],
          title: "First Papers at the Registry",
          giver_id: "#11",
          giver_name: "Maren Halvorsen",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          can_finish_now: true,
          ready_to_finish: true,
          finish_cmd: "+quest/finish #18 tok1",
          abandon_cmd: "+quest/abandon q1",
          checkpoints: [],
        },
      ],
      available: [
        {
          key: "side.one_shot",
          title: "One-Shot Charter Audit",
          giver_id: "#18",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          allow_restart: false,
          accept_cmd: "+quest/accept #18 tok_side",
          decline_cmd: "+quest/decline #18 tok_side",
          introduction: "Audit one ledger entry.",
          reward_summary: "250 cr",
          checkpoints: [],
        },
        {
          key: GUIDED_TOUR_KEYS[1],
          title: "Stock the Locker and Tanks",
          giver_id: "#18",
          giver_name: "Ilse Oduya-Brandt",
          turn_in_id: "#18",
          turn_in_name: "Ilse Oduya-Brandt",
          allow_restart: true,
          accept_cmd: "+quest/accept #18 tok_g2",
          decline_cmd: "",
          introduction: "Guided step 2.",
          reward_summary: "Guided Step 2/7",
          checkpoints: [],
        },
      ],
      completed: [],
    });

    const sent: string[] = [];
    const app = { quests, tacsim: { trade: null }, focusComposer: vi.fn() } as unknown as GameApp;

    // 1. Open NPC dialog for Ilse -> turn-in quest selected first, crest is '?' turn_in
    quests.openNpcDialog("#18");
    const first = render(QuestDialogModal, { app, onCmd: (c) => sent.push(c) });
    const getBadge = () => first.container.querySelector(".qbadge") as HTMLElement;
    expect(getBadge().dataset.marker).toBe("turn_in");
    expect(getBadge().textContent?.trim()).toBe("?");

    // Arm abandon on turn-in quest, then switch to the [!] offer tab -> crest switches to '!' and confirm resets
    await fireEvent.click(screen.getByRole("button", { name: "Abandon Quest" }));
    expect(screen.getByRole("button", { name: "Confirm Abandon" })).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: /One-Shot Charter Audit/ }));
    expect(getBadge().dataset.marker).toBe("available");
    expect(getBadge().textContent?.trim()).toBe("!");

    // Switch back to turn-in quest -> abandon button must have reset (not armed)
    await fireEvent.click(screen.getByRole("button", { name: /First Papers at the Registry/ }));
    expect(screen.getByRole("button", { name: "Abandon Quest" })).toBeTruthy();
    first.unmount();

    // 2. Quest Log Available tab shows 2-click Decline Permanently ONLY for non-restartable offers
    quests.openLog("available");
    render(QuestDialogModal, { app, onCmd: (c) => sent.push(c) });
    const declineBtns = screen.getAllByRole("button", { name: "Decline Permanently" });
    expect(declineBtns).toHaveLength(1);
    await fireEvent.click(declineBtns[0]!);
    const confirmDecline = screen.getByRole("button", { name: "Confirm Permanent Decline" });
    await fireEvent.click(confirmDecline);
    expect(sent).toContain("+quest/decline #18 tok_side");
  });
});
